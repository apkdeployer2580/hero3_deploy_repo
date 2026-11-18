use alloc::{borrow::ToOwned, boxed::Box, collections::BTreeMap, format, string::String, sync::Arc, vec, vec::Vec};

use jvm::{
    Jvm,
    runtime::{JavaIoInputStream, JavaLangClassLoader},
};
use spin::Mutex;
use wipi_types::wipic::{WIPICIndirectPtr, WIPICWord};

use wie_backend::{AsyncCallable, Event, Instant, System};
use wie_core_arm::{Allocator, ArmCore};
use wie_util::{ByteRead, ByteWrite, Result, WieError, read_generic, write_generic};
use wie_wipi_c::{WIPICContext, WIPICMethodBody};

// mostly same as ktf's one, can we merge those?
#[derive(Clone)]
pub struct LgtWIPICContext {
    core: ArmCore,
    system: System,
    jvm: Jvm,
}

impl LgtWIPICContext {
    pub fn new(core: ArmCore, system: System, jvm: Jvm) -> Self {
        Self { core, system, jvm }
    }

    /// One jar resource, fetched at most once per process. `None` = no such resource in
    /// the jar (and that answer is cached too, so a name the game probes repeatedly does
    /// not keep rebuilding the archive to be told "no" again).
    async fn resource(&self, name: &str) -> Option<Arc<Vec<u8>>> {
        let key = (self.system.aid().to_owned(), name.to_owned());
        if let Some(hit) = resource_cached(&key) {
            return hit;
        }

        let class_loader = self.jvm.current_class_loader().await.unwrap();
        let stream = JavaLangClassLoader::get_resource_as_stream(&self.jvm, &class_loader, name).await.unwrap();
        let value = match stream {
            None => None,
            Some(stream) => Some(Arc::new(JavaIoInputStream::read_until_end(&self.jvm, &stream).await.unwrap())),
        };

        resource_store(key, value.clone());
        value
    }
}

// ── MC_knlGetResource cache ──────────────────────────────────────────────────
//
// ★★★ 2026-08-29 (72차) — THE 49차 FIX, WHICH ONLY EVER LANDED IN `wie_ktf`.
//
// This file's own header says "mostly same as ktf's one, can we merge those?", and the
// half that was NOT the same was the cache. LGT — the build we actually ship — has been
// re-fetching every resource from the jar on every request since 49차, and the player
// report that found it is *"인물과의 대사가 가끔 살짝씩 프레임드랍이 걸리는듯한 느낌"*.
//
// Why dialogue specifically: the guest's portrait routine (`0x14aa0`) compares the wanted
// face id against a cached byte and, when it differs, frees the old image, loads
// `res/npc/face_bm` in full, extracts one sub-image and frees the file buffer again. That
// sheet is 139,500 bytes — the second-largest entry in the jar — and a two-speaker
// conversation reloads it on nearly every page.
//
// What one reload cost without this cache: `get_resource_size` and `read_resource` each
// call `JavaLangClassLoader::get_resource_as_stream`, which is `getResource` + `openStream`
// and reaches `get_zip_archive` THREE times (findResource, getEntry, getInputStream). Each
// of those copies the whole 1,862,044-byte jar into a fresh `Vec` and re-parses a
// 1,290-entry central directory. Two calls per resource = six full-jar copies (~11 MB of
// memcpy) plus six central-directory parses, then the 139.5 KB comes out of
// `JavaIoInputStream::read_until_end` in 1024-byte chunks through string-keyed virtual
// dispatch. All of it inside `emulator.tick()`. 49차 measured the same shape at 3.9 ms on
// a desktop x86, in a frame that budgets 24 ms — on a phone core, several times that.
//
// Jar entries are immutable for the life of the process, so one fetch per name is enough.
// The cache is a module global rather than a field because a fresh `LgtWIPICContext` is
// constructed for EVERY WIPI-C call, so an instance field would never see a second hit.
// Keyed by `(aid, name)` so two emulators in one process cannot read each other's data.
//
// ⛔ The `system.filesystem()` fallback below is NOT cached and must not be: the jar is
// immutable, the platform filesystem is not.
type ResourceKey = (String, String);
static RESOURCE_CACHE: Mutex<Option<BTreeMap<ResourceKey, Option<Arc<Vec<u8>>>>>> = Mutex::new(None);
static RESOURCE_CACHE_BYTES: Mutex<usize> = Mutex::new(0);

/// Cache ceiling. Hero3's whole uncompressed jar is 3.42 MB, so this holds everything the
/// game can ask for and still refuses to grow without bound for a larger archive. Past it
/// nothing new is cached and the old fetch-every-time path is used, so the worst case is
/// exactly the behaviour this replaces.
const RESOURCE_CACHE_MAX: usize = 8 * 1024 * 1024;

/// `Some(entry)` on a hit — the inner `Option` distinguishes "cached: this resource does
/// not exist" from "cached: here are its bytes", so a miss is not re-fetched either.
/// `None` means we have never looked.
fn resource_cached(key: &ResourceKey) -> Option<Option<Arc<Vec<u8>>>> {
    RESOURCE_CACHE.lock().as_ref().and_then(|m| m.get(key).cloned())
}

fn resource_store(key: ResourceKey, value: Option<Arc<Vec<u8>>>) {
    let len = value.as_ref().map(|v| v.len()).unwrap_or(0);
    let mut bytes = RESOURCE_CACHE_BYTES.lock();
    if *bytes + len > RESOURCE_CACHE_MAX {
        return;
    }
    *bytes += len;
    RESOURCE_CACHE.lock().get_or_insert_with(BTreeMap::new).insert(key, value);
}

#[async_trait::async_trait]
impl WIPICContext for LgtWIPICContext {
    fn alloc_raw(&mut self, size: WIPICWord) -> Result<WIPICWord> {
        Allocator::alloc(&mut self.core, size)
    }

    fn alloc(&mut self, size: WIPICWord) -> Result<WIPICIndirectPtr> {
        let address = Allocator::alloc(&mut self.core, size + size_of::<WIPICWord>() as WIPICWord)?;
        write_generic(&mut self.core, address, size)?;

        Ok(WIPICIndirectPtr(address + size_of::<WIPICWord>() as WIPICWord))
    }

    fn free(&mut self, memory: WIPICIndirectPtr) -> Result<()> {
        let base_address = memory.0 - size_of::<WIPICWord>() as WIPICWord;

        let size: WIPICWord = read_generic(&self.core, base_address)?;
        Allocator::free(&mut self.core, base_address, size + size_of::<WIPICWord>() as WIPICWord)
    }

    fn free_raw(&mut self, address: WIPICWord, size: WIPICWord) -> Result<()> {
        Allocator::free(&mut self.core, address, size)?;

        Ok(())
    }

    fn data_ptr(&self, memory: WIPICIndirectPtr) -> Result<WIPICWord> {
        Ok(memory.0)
    }

    fn system(&mut self) -> &mut System {
        &mut self.system
    }

    async fn call_function(&mut self, address: WIPICWord, args: &[WIPICWord]) -> Result<WIPICWord> {
        self.core.run_function(address, args).await
    }

    fn spawn(&mut self, callback: WIPICMethodBody) -> Result<()> {
        struct SpawnProxy {
            context: LgtWIPICContext,
            callback: WIPICMethodBody,
        }

        impl AsyncCallable<Result<()>> for SpawnProxy {
            async fn call(mut self) -> Result<()> {
                self.context.jvm.attach_thread().unwrap();
                self.callback.call(&mut self.context, Box::new([])).await?;
                self.context.jvm.detach_thread().unwrap();

                Ok(())
            }
        }

        self.system.spawn(SpawnProxy {
            context: self.clone(),
            callback,
        });

        Ok(())
    }

    async fn get_resource_size(&self, name: &str) -> Result<Option<usize>> {
        if let Some(data) = self.resource(name).await {
            return Ok(Some(data.len()));
        }

        Ok(self.system.filesystem().size(name).await)
    }

    async fn read_resource(&self, name: &str) -> Result<Vec<u8>> {
        if let Some(data) = self.resource(name).await {
            return Ok(data.as_ref().clone());
        }

        let Some(size) = self.system.filesystem().size(name).await else {
            return Err(WieError::FatalError(format!("Missing resource: {name}")));
        };
        let mut data = vec![0; size];
        let read = self.system.filesystem().read(name, 0, size, &mut data).await.unwrap_or(0);
        data.truncate(read);

        Ok(data)
    }

    fn set_timer(&mut self, ptr_timer: WIPICWord, due: Instant, callback: WIPICMethodBody) {
        let context = self.clone();

        // Keyed by the guest's timer object: re-arming REPLACES the pending shot, and
        // MC_knlUnsetTimer can cancel it. Without this the queue accumulated one live
        // timer per screen the guest ever visited (PLAY_FEEDBACK_FIXES.md §41 item 7).
        let key = self.system().event_queue().arm_timer(ptr_timer as u64);
        self.system().event_queue().push(Event::timer_keyed(due, key, move || {
            let mut context = context.clone();

            async move {
                callback.call(&mut context, Box::new([])).await?;
                Ok(())
            }
        }))
    }

    fn unset_timer(&mut self, ptr_timer: WIPICWord) {
        self.system().event_queue().cancel_timer(ptr_timer as u64);
    }
}

impl ByteRead for LgtWIPICContext {
    fn read_bytes(&self, address: WIPICWord, result: &mut [u8]) -> wie_util::Result<usize> {
        self.core.read_bytes(address, result)
    }
}

impl ByteWrite for LgtWIPICContext {
    fn write_bytes(&mut self, address: WIPICWord, data: &[u8]) -> wie_util::Result<()> {
        self.core.write_bytes(address, data)
    }
}
