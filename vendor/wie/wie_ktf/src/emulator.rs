use core::pin::Pin;

use alloc::{borrow::ToOwned, boxed::Box, collections::BTreeMap, format, string::String, sync::Arc, vec, vec::Vec};

use jvm::{ClassInstance, Jvm, Result as JvmResult, runtime::JavaLangString};
use spin::Mutex;

use wie_backend::{Emulator, Event, Options, Platform, System, TaskRunner};
use wie_core_arm::{Allocator, ArmCore};
use wie_util::{Result, WieError};

use crate::{
    adf::{KtfAdf, find_client_bin},
    runtime::{KtfJvmSupport, oom},
};

pub const IMAGE_BASE: u32 = 0x100000;

struct KtfTaskRunner {
    core: ArmCore,
}

#[async_trait::async_trait]
impl TaskRunner for KtfTaskRunner {
    async fn run(&self, future: Pin<Box<dyn Future<Output = Result<()>> + Send>>) -> Result<()> {
        self.core.run_in_thread(async move || future.await)?.await
    }
}

pub struct KtfEmulator {
    core: ArmCore,
    system: System,
    /// The JVM, published by `start` once it exists.
    ///
    /// ★ Needed by HOST-INITIATED guest calls. The JVM identifies a "thread" by the
    /// executor TASK id (`WieJavaRuntime::current_task_id`), and `Jvm::find_calling_class`
    /// does `threads.get(&thread_id).unwrap()`. A `System::spawn`ed task is a brand-new
    /// task id, so unless it attaches first, any JVM path the guest code reaches panics
    /// on an unregistered thread. `KtfWIPICContext::spawn` has always wrapped its callback
    /// in `attach_thread`/`detach_thread` for exactly this reason; `call_guest_function`
    /// did not, which is why it had never actually worked on a device.
    jvm: Arc<Mutex<Option<Jvm>>>,
}

impl KtfEmulator {
    pub fn from_archive(platform: Box<dyn Platform>, files: BTreeMap<String, Vec<u8>>, options: Options) -> Result<Self> {
        let adf = files
            .get("__adf__")
            .ok_or_else(|| WieError::FatalError("Missing __adf__ in KTF archive".into()))?;
        let adf = KtfAdf::parse(adf);

        tracing::info!("Loading app {}, pid {}, mclass {}", adf.aid, adf.pid, adf.mclass);

        let jar_filename = format!("{}.jar", adf.aid);

        Self::load(platform, &jar_filename, &adf.pid, &adf.aid, Some(adf.mclass), &files, options)
    }

    pub fn from_jar(
        platform: Box<dyn Platform>,
        jar_filename: &str,
        jar: Vec<u8>,
        pid: &str,
        aid: &str,
        main_class_name: Option<String>,
        options: Options,
    ) -> Result<Self> {
        let files = [(jar_filename.to_owned(), jar)].into_iter().collect();

        Self::load(platform, jar_filename, pid, aid, main_class_name, &files, options)
    }

    pub fn loadable_archive(files: &BTreeMap<String, Vec<u8>>) -> bool {
        files.contains_key("__adf__")
    }

    pub fn loadable_jar(jar: &[u8]) -> bool {
        find_client_bin(jar).is_ok()
    }

    fn load(
        platform: Box<dyn Platform>,
        jar_filename: &str,
        pid: &str,
        aid: &str,
        main_class_name: Option<String>,
        files: &BTreeMap<String, Vec<u8>>,
        mut options: Options,
    ) -> Result<Self> {
        let mut core = ArmCore::new(options.enable_gdbserver, options.profile.take())?;
        let system = System::new(platform, pid, aid, KtfTaskRunner { core: core.clone() });

        for (path, data) in files {
            let path = path.trim_start_matches("P/");
            system.filesystem().add_virtual(path, data.clone());
        }

        Allocator::init(&mut core)?;

        let mut core_clone = core.clone();
        let mut system_clone = system.clone();
        let jar_filename_clone = jar_filename.to_owned();
        let jvm_slot: Arc<Mutex<Option<Jvm>>> = Arc::new(Mutex::new(None));
        let jvm_slot_clone = jvm_slot.clone();

        system.spawn(async move || Self::start(&mut core_clone, &mut system_clone, jar_filename_clone, main_class_name, jvm_slot_clone).await);

        Ok(Self { core, system, jvm: jvm_slot })
    }

    #[tracing::instrument(name = "start", skip_all)]
    async fn start(
        core: &mut ArmCore,
        system: &mut System,
        jar_filename: String,
        main_class_name: Option<String>,
        jvm_slot: Arc<Mutex<Option<Jvm>>>,
    ) -> Result<()> {
        let (jvm, class_loader) = KtfJvmSupport::init(core, system, Some(&jar_filename)).await?;
        // Publish it for host-initiated guest calls (see the `jvm` field).
        *jvm_slot.lock() = Some(jvm.clone());

        let main_class_name = if let Some(x) = main_class_name {
            x
        } else {
            return Err(WieError::FatalError("Main class not found".into()));
        };

        let main_class_name = main_class_name.replace('.', "/");

        let main_class_name_java = JavaLangString::from_rust_string(&jvm, &main_class_name).await.unwrap();
        let _main_class: Box<dyn ClassInstance> = jvm
            .invoke_virtual(
                &class_loader,
                "loadClass",
                "(Ljava/lang/String;)Ljava/lang/Class;",
                (main_class_name_java.clone(),),
            )
            .await
            .unwrap();

        let mut args_array = jvm.instantiate_array("Ljava/lang/String;", 1).await.unwrap();
        jvm.store_array(&mut args_array, 0, vec![main_class_name_java]).await.unwrap();
        let result: JvmResult<()> = jvm
            .invoke_static("org/kwis/msp/lcdui/Main", "main", "([Ljava/lang/String;)V", (args_array,))
            .await;

        if let Err(x) = result {
            // `oom::to_wie_err` first: building the stack trace allocates, so an out-of-memory exception reaching here would panic inside
            // `JvmSupport::to_wie_err` instead of being reported.
            return Err(oom::to_wie_err(&jvm, x).await);
        }

        Ok(())
    }
}

impl Emulator for KtfEmulator {
    fn handle_event(&mut self, event: Event) {
        self.system.event_queue().push(event)
    }

    fn next_wakeup_ms(&self) -> Option<u64> {
        self.system.next_wakeup_ms()
    }

    fn tick(&mut self) -> Result<()> {
        self.system.tick().map_err(|x| {
            let reg_stack = self.core.dump_reg_stack(IMAGE_BASE);
            match x {
                WieError::FatalError(msg) => WieError::FatalError(format!("{msg}\n{reg_stack}")),
                _ => WieError::FatalError(format!("{x}\n{reg_stack}")),
            }
        })
    }

    fn read_memory(&self, address: u32, len: usize) -> Vec<u8> {
        use wie_util::ByteRead;
        let mut buf = alloc::vec![0u8; len];
        match self.core.read_bytes(address, &mut buf) {
            Ok(n) => {
                buf.truncate(n);
                buf
            }
            Err(_) => Vec::new(),
        }
    }

    fn write_memory(&mut self, address: u32, data: &[u8]) -> bool {
        use wie_util::ByteWrite;
        self.core.write_bytes(address, data).is_ok()
    }

    fn call_guest_function_out(&mut self, address: u32, args: &[u32], out: alloc::sync::Arc<wie_backend::GuestCallResult>) {
        // Same as `call_guest_function`, plus the return value published for the host.
        let mut core = self.core.clone();
        let args: Vec<u32> = args.to_vec();
        let jvm = self.jvm.lock().clone();
        self.system.spawn(async move || {
            let _attached = JvmThreadGuard::attach(jvm);
            match core.run_function::<u32>(address, &args).await {
                Ok(ret) => {
                    tracing::debug!("guest call {address:#x} -> {ret:#x}");
                    out.ok(ret);
                }
                Err(e) => {
                    tracing::warn!("guest call {address:#x} failed: {e:?}");
                    out.fail();
                }
            }
            Ok(())
        });
    }

    fn call_guest_function(&mut self, address: u32, args: &[u32]) {
        // Spawn a cooperative task that runs the guest function to completion on
        // its own ARM thread context (fresh stack), the same mechanism WIPI
        // callbacks use (`KtfWIPICContext::call_function` -> `run_function`).
        // `run_function` saves/restores CPU context, so this never disturbs the
        // parked main-loop task; it executes on the next `System::tick`.
        let mut core = self.core.clone();
        let args: Vec<u32> = args.to_vec();
        let jvm = self.jvm.lock().clone();
        self.system.spawn(async move || {
            let _attached = JvmThreadGuard::attach(jvm);
            match core.run_function::<u32>(address, &args).await {
                Ok(ret) => tracing::debug!("guest call {address:#x} -> {ret:#x}"),
                Err(e) => tracing::warn!("guest call {address:#x} failed: {e:?}"),
            }
            Ok(())
        });
    }
}

/// Registers the CURRENT executor task as a JVM thread for as long as it lives.
///
/// A `System::spawn`ed task gets a fresh task id, and the JVM keys its per-thread state
/// (the Java frame stack that `find_calling_class` reads) by exactly that id. Guest code
/// reached through a WIPI vtable can land in the JVM, and there it would index a thread
/// map that has no entry — `threads.get(&id).unwrap()`, i.e. a panic that kills the app.
///
/// `KtfWIPICContext::spawn` has always done this around its callbacks. Host-initiated
/// calls need it for the same reason, and a guard makes the detach unmissable on every
/// exit path (including the error paths, which is where the hand-written version of this
/// would have leaked a thread entry).
struct JvmThreadGuard(Option<Jvm>);

impl JvmThreadGuard {
    fn attach(jvm: Option<Jvm>) -> Self {
        // `None` = the JVM does not exist yet (a call issued during boot). Running the
        // guest function anyway is right: not every target touches the JVM, and refusing
        // would be a silent no-op.
        if let Some(jvm) = &jvm
            && let Err(e) = jvm.attach_thread()
        {
            tracing::warn!("could not attach a JVM thread for a host-initiated guest call: {e:?}");
            return Self(None);
        }
        Self(jvm)
    }
}

impl Drop for JvmThreadGuard {
    fn drop(&mut self) {
        if let Some(jvm) = &self.0
            && let Err(e) = jvm.detach_thread()
        {
            tracing::warn!("could not detach the JVM thread after a host-initiated guest call: {e:?}");
        }
    }
}
