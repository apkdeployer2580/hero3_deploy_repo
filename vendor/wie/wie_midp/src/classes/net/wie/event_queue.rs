use alloc::{string::ToString, vec, vec::Vec};

use futures::TryFutureExt;
use java_class_proto::{JavaFieldProto, JavaMethodProto};
use java_constants::{FieldAccessFlags, MethodAccessFlags};
use java_runtime::classes::java::lang::Runnable;
use jvm::{Array, ClassInstanceRef, Jvm, Result as JvmResult};

use wie_backend::{Event, KeyCode};
use wie_jvm_support::{WieJavaClassProto, WieJvmContext};

use crate::classes::javax::microedition::midlet::MIDlet;

#[repr(i32)]
#[allow(clippy::enum_variant_names)]
enum EventQueueEvent {
    // TODO it's wipi event codes
    KeyEvent = 1,
    RepaintEvent = 41,
    NotifyEvent = 1000,
}

impl EventQueueEvent {
    fn from_raw(raw: i32) -> Option<Self> {
        Some(match raw {
            x if x == Self::KeyEvent as i32 => Self::KeyEvent,
            x if x == Self::RepaintEvent as i32 => Self::RepaintEvent,
            x if x == Self::NotifyEvent as i32 => Self::NotifyEvent,
            _ => return None,
        })
    }
}

#[repr(i32)]
#[derive(Debug)]
#[allow(dead_code, clippy::enum_variant_names)]
pub enum KeyboardEventType {
    KeyPressed = 1,
    KeyReleased = 2,
    KeyRepeated = 3,
    KeyTyped = 4,
}

impl KeyboardEventType {
    pub fn from_raw(raw: i32) -> Option<Self> {
        Some(match raw {
            x if x == Self::KeyPressed as i32 => Self::KeyPressed,
            x if x == Self::KeyReleased as i32 => Self::KeyReleased,
            x if x == Self::KeyRepeated as i32 => Self::KeyRepeated,
            x if x == Self::KeyTyped as i32 => Self::KeyTyped,
            _ => return None,
        })
    }
}

#[repr(i32)]
#[allow(clippy::upper_case_acronyms)]
#[allow(non_camel_case_types)]
pub enum MIDPKeyCode {
    // keycode is for skvm
    UP = 141, // MIDP Canvas's name
    DOWN = 146,
    LEFT = 142,
    RIGHT = 145,
    FIRE = 148,
    LEFT_SOFT_KEY = 6,
    RIGHT_SOFT_KEY = 7,
    CLEAR = 8,
    CALL = 10,
    HANGUP = -1,
    VOLUME_UP = 13,
    VOLUME_DOWN = 14,

    KEY_NUM0 = 48,
    KEY_NUM1 = 49,
    KEY_NUM2 = 50,
    KEY_NUM3 = 51,
    KEY_NUM4 = 52,
    KEY_NUM5 = 53,
    KEY_NUM6 = 54,
    KEY_NUM7 = 55,
    KEY_NUM8 = 56,
    KEY_NUM9 = 57,
    KEY_POUND = 35, // #
    KEY_STAR = 42,  // *
}

impl MIDPKeyCode {
    pub fn from_raw(raw: i32) -> Option<Self> {
        Some(match raw {
            x if x == Self::UP as i32 => Self::UP,
            x if x == Self::DOWN as i32 => Self::DOWN,
            x if x == Self::LEFT as i32 => Self::LEFT,
            x if x == Self::RIGHT as i32 => Self::RIGHT,
            x if x == Self::FIRE as i32 => Self::FIRE,
            x if x == Self::LEFT_SOFT_KEY as i32 => Self::LEFT_SOFT_KEY,
            x if x == Self::RIGHT_SOFT_KEY as i32 => Self::RIGHT_SOFT_KEY,
            x if x == Self::CLEAR as i32 => Self::CLEAR,
            x if x == Self::CALL as i32 => Self::CALL,
            x if x == Self::HANGUP as i32 => Self::HANGUP,
            x if x == Self::VOLUME_UP as i32 => Self::VOLUME_UP,
            x if x == Self::VOLUME_DOWN as i32 => Self::VOLUME_DOWN,
            x if x == Self::KEY_NUM0 as i32 => Self::KEY_NUM0,
            x if x == Self::KEY_NUM1 as i32 => Self::KEY_NUM1,
            x if x == Self::KEY_NUM2 as i32 => Self::KEY_NUM2,
            x if x == Self::KEY_NUM3 as i32 => Self::KEY_NUM3,
            x if x == Self::KEY_NUM4 as i32 => Self::KEY_NUM4,
            x if x == Self::KEY_NUM5 as i32 => Self::KEY_NUM5,
            x if x == Self::KEY_NUM6 as i32 => Self::KEY_NUM6,
            x if x == Self::KEY_NUM7 as i32 => Self::KEY_NUM7,
            x if x == Self::KEY_NUM8 as i32 => Self::KEY_NUM8,
            x if x == Self::KEY_NUM9 as i32 => Self::KEY_NUM9,
            x if x == Self::KEY_POUND as i32 => Self::KEY_POUND,
            x if x == Self::KEY_STAR as i32 => Self::KEY_STAR,
            _ => return None,
        })
    }

    fn from_key_code(keycode: KeyCode) -> Self {
        match keycode {
            KeyCode::UP => Self::UP,
            KeyCode::DOWN => Self::DOWN,
            KeyCode::LEFT => Self::LEFT,
            KeyCode::RIGHT => Self::RIGHT,
            KeyCode::OK => Self::FIRE,
            KeyCode::LEFT_SOFT_KEY => Self::LEFT_SOFT_KEY,
            KeyCode::RIGHT_SOFT_KEY => Self::RIGHT_SOFT_KEY,
            KeyCode::CLEAR => Self::CLEAR,
            KeyCode::CALL => Self::CALL,
            KeyCode::HANGUP => Self::HANGUP,
            KeyCode::VOLUME_UP => Self::VOLUME_UP,
            KeyCode::VOLUME_DOWN => Self::VOLUME_DOWN,
            KeyCode::NUM0 => Self::KEY_NUM0,
            KeyCode::NUM1 => Self::KEY_NUM1,
            KeyCode::NUM2 => Self::KEY_NUM2,
            KeyCode::NUM3 => Self::KEY_NUM3,
            KeyCode::NUM4 => Self::KEY_NUM4,
            KeyCode::NUM5 => Self::KEY_NUM5,
            KeyCode::NUM6 => Self::KEY_NUM6,
            KeyCode::NUM7 => Self::KEY_NUM7,
            KeyCode::NUM8 => Self::KEY_NUM8,
            KeyCode::NUM9 => Self::KEY_NUM9,
            KeyCode::HASH => Self::KEY_POUND,
            KeyCode::STAR => Self::KEY_STAR,
        }
    }
}

// class net.wie.EventQueue
pub struct EventQueue;

impl EventQueue {
    pub fn as_proto() -> WieJavaClassProto {
        WieJavaClassProto {
            name: "net/wie/EventQueue",
            parent_class: Some("java/lang/Object"),
            interfaces: vec![],
            methods: vec![
                JavaMethodProto::new("<init>", "()V", Self::init, Default::default()),
                JavaMethodProto::new("getNextEvent", "([I)V", Self::get_next_event, Default::default()),
                JavaMethodProto::new("dispatchEvent", "([I)V", Self::dispatch_event, Default::default()),
                JavaMethodProto::new("callSerially", "(Ljava/lang/Runnable;)V", Self::call_serially, Default::default()),
                JavaMethodProto::new(
                    "getEventQueue",
                    "()Lnet/wie/EventQueue;",
                    Self::get_event_queue,
                    MethodAccessFlags::STATIC,
                ),
            ],
            fields: vec![
                JavaFieldProto::new("eventQueue", "Lnet/wie/EventQueue;", FieldAccessFlags::STATIC),
                JavaFieldProto::new("callSeriallyEvents", "Ljava/util/Vector;", Default::default()),
            ],
            access_flags: Default::default(),
        }
    }

    async fn init(jvm: &Jvm, _context: &mut WieJvmContext, mut this: ClassInstanceRef<Self>) -> JvmResult<()> {
        tracing::debug!("net.wie.EventQueue::<init>({this:?})");

        let _: () = jvm.invoke_special(&this, "java/lang/Object", "<init>", "()V", ()).await?;

        let call_serially_events = jvm.new_class("java/util/Vector", "()V", ()).await?;
        jvm.put_field(&mut this, "callSeriallyEvents", "Ljava/util/Vector;", call_serially_events)
            .await?;

        Ok(())
    }

    // TODO this resembles WIPI's architecture for now, but we need to change it to event listener
    async fn get_next_event(
        jvm: &Jvm,
        context: &mut WieJvmContext,
        this: ClassInstanceRef<Self>,
        mut event: ClassInstanceRef<Array<i32>>,
    ) -> JvmResult<()> {
        tracing::debug!("net.wie.EventQueue::getNextEvent({this:?}, {event:?})");

        let mut pending_timer_events = Vec::new();
        // ★ 2026-08-09. Bound on how many DUE timer callbacks may run back to back
        // before this function hands control back to the executor.
        //
        // Without it, a device that cannot finish a frame inside the game's own timer
        // interval (~21 ms, measured) never leaves this loop: the callback re-arms the
        // timer, the re-armed timer is already due by the time the callback returns, so
        // `pop` keeps yielding a due `Event::Timer` and the queue is never empty — which
        // is the ONLY branch below that reaches `sleep(16)`. `getNextEvent` then does not
        // return, so neither does `emulator.tick()`.
        //
        // That is worse than slow. `Executor::tick`'s 8 ms deadline is only tested
        // BETWEEN `step()` calls, so it is bypassed; and back in `wie_android`'s worker
        // loop, everything that lives outside `tick()` stops happening — the input drain,
        // the `paused` check that stops the emulator when the app is backgrounded, and the
        // whole `FRAME_BUDGET_MS` pacing. Input latency becomes unbounded and a
        // backgrounded game keeps burning the battery.
        //
        // A yield costs one extra executor `step`. On a device that keeps up this branch
        // runs 0 or 1 due timers per call, so the bound is never reached and behaviour is
        // unchanged; on a device that does not, it converts "stops responding" into
        // "runs slowly", which is the whole point of this round.
        /// ★ 1, not 4 (measured 2026-08-09 on the Galaxy S9). At 4 the average
        /// `emulator.tick()` was **29.9 ms** and 895 of 1807 ticks blew the 24 ms frame
        /// budget, because one poll was chaining up to four whole guest frames on a
        /// device where each one costs ~8 ms. Everything the worker does BETWEEN ticks —
        /// draining input, the background/`paused` check, the frame pacing — was waiting
        /// on that. One frame per poll is what "hand control back after each frame"
        /// actually means, and on a device that keeps up this branch already runs 0 or 1
        /// times per call, so nothing changes there.
        const MAX_TIMERS_PER_POLL: u32 = 1;
        let mut timers_run: u32 = 0;
        loop {
            let now = context.system().platform().now();
            let maybe_event = context.system().event_queue().pop();

            if let Some(x) = maybe_event {
                let event_data = match x {
                    Event::Redraw => vec![EventQueueEvent::RepaintEvent as _, 0, 0, 0],
                    Event::Keydown(x) => vec![
                        EventQueueEvent::KeyEvent as _,
                        KeyboardEventType::KeyPressed as _,
                        MIDPKeyCode::from_key_code(x) as _,
                        0,
                    ],
                    Event::Keyup(x) => vec![
                        EventQueueEvent::KeyEvent as _,
                        KeyboardEventType::KeyReleased as _,
                        MIDPKeyCode::from_key_code(x) as _,
                        0,
                    ],
                    Event::Keyrepeat(x) => vec![
                        EventQueueEvent::KeyEvent as _,
                        KeyboardEventType::KeyRepeated as _,
                        MIDPKeyCode::from_key_code(x) as _,
                        0,
                    ],
                    Event::Timer { due, key, callback } => {
                        // A keyed shot fires only while it is still the LIVE one for its
                        // timer object — a re-arm (MC_knlSetTimer on the same ptr_timer)
                        // or a cancel (MC_knlUnsetTimer) that happened after it was queued
                        // makes it stale, and stale shots are discarded here instead of
                        // run. This is the other half of timer cancellation: the queue
                        // cannot be edited while a guest callback (possibly the caller of
                        // MC_knlUnsetTimer itself) is running, so cancellation marks and
                        // this drop enforces. PLAY_FEEDBACK_FIXES.md §41 item 7.
                        if let Some(key) = key
                            && !context.system().event_queue().timer_key_current(key)
                        {
                            wie_util::stats::note_timer_stale_drop();
                            continue;
                        }

                        // TODO we should wait for timer more efficiently
                        if due < now {
                            callback()
                                .or_else(async |x| Err(jvm.exception("net/wie/WieError", &x.to_string()).await))
                                .await?;
                            timers_run += 1;
                            if timers_run >= MAX_TIMERS_PER_POLL {
                                timers_run = 0;
                                // Hands the executor back its `step` boundary. The timers
                                // stashed in `pending_timer_events` stay stashed across
                                // this — nothing else consumes them, and they are pushed
                                // back on every path out of the loop.
                                context.system().yield_now().await;
                            }
                        } else {
                            // push it to event queue again
                            pending_timer_events.push(Event::Timer { due, key, callback });
                        }

                        continue;
                    }
                    // wipi notifyEvent
                    Event::Notify { r#type, param1, param2 } => vec![EventQueueEvent::NotifyEvent as i32, r#type, param1, param2],
                };

                jvm.store_array(&mut event, 0, event_data).await?;

                break;
            } else {
                let call_serially_events = jvm.get_field(&this, "callSeriallyEvents", "Ljava/util/Vector;").await?;
                if !jvm.invoke_virtual(&call_serially_events, "isEmpty", "()Z", ()).await? {
                    let event: ClassInstanceRef<Runnable> =
                        jvm.invoke_virtual(&call_serially_events, "remove", "(I)Ljava/lang/Object;", (0,)).await?;
                    let _: () = jvm.invoke_virtual(&event, "run", "()V", ()).await?;
                }

                // ★ SLEEP UNTIL THE NEXT TIMER IS DUE, not a flat 16 ms (2026-08-13,
                // `MIGRATION_LGT.md` §2.24).
                //
                // Everything in `pending_timer_events` is a timer this loop looked at and
                // found NOT yet due, so their earliest `due` is exactly when there will be
                // work again — and Hero3 re-arms its frame timer at ~21 ms. Sleeping a flat
                // 16 ms quantises that to the next multiple: the timer becomes due at 21 ms,
                // is missed by the 16 ms wake-up, and only runs at 32 ms — a 25 fps game
                // capped at 31 fps before any work, and far worse once a frame's work pushes
                // the next look further out (measured 7.5 fps on the panel screens with the
                // host thread 79 % IDLE).
                //
                // The 16 ms stays as the CEILING, so a queue with no timers behaves exactly
                // as before, and 1 ms as the floor so this can never become a spin.
                let now = context.system().platform().now();
                let next_due = pending_timer_events
                    .iter()
                    .filter_map(|event| match event {
                        Event::Timer { due, .. } => Some(*due),
                        _ => None,
                    })
                    .min();
                let nap = match next_due {
                    Some(due) if due > now => (due - now).clamp(1, 16),
                    // Already due (or nothing pending): the shortest hop that still hands
                    // the executor its `step` boundary back.
                    Some(_) => 1,
                    None => 16,
                };
                wie_util::stats::note_pump_nap(nap);
                context.system().sleep(nap).await;

                for event in pending_timer_events.drain(..) {
                    context.system().event_queue().push(event);
                }
            }
        }

        for event in pending_timer_events {
            context.system().event_queue().push(event);
        }

        Ok(())
    }

    async fn dispatch_event(
        jvm: &Jvm,
        _context: &mut WieJvmContext,
        this: ClassInstanceRef<Self>,
        event: ClassInstanceRef<Array<i32>>,
    ) -> JvmResult<()> {
        tracing::debug!("net.wie.EventQueue::dispatchEvent({this:?}, {event:?})");

        let current_midlet: ClassInstanceRef<MIDlet> = jvm
            .get_static_field("javax/microedition/midlet/MIDlet", "currentMIDlet", "Ljavax/microedition/midlet/MIDlet;")
            .await?;

        let display = jvm
            .invoke_static(
                "javax/microedition/lcdui/Display",
                "getDisplay",
                "(Ljavax/microedition/midlet/MIDlet;)Ljavax/microedition/lcdui/Display;",
                (current_midlet,),
            )
            .await?;

        let event = jvm.load_array(&event, 0, 4).await?;
        let event_kind = if let Some(event_kind) = EventQueueEvent::from_raw(event[0]) {
            event_kind
        } else {
            return Err(jvm
                .exception("java/lang/IllegalArgumentException", "Invalid event queue event type")
                .await);
        };

        match event_kind {
            EventQueueEvent::RepaintEvent => {
                let _: () = jvm.invoke_virtual(&display, "handlePaintEvent", "()V", ()).await?;
            }
            EventQueueEvent::KeyEvent => {
                let event_type = if let Some(event_type) = KeyboardEventType::from_raw(event[1]) {
                    event_type
                } else {
                    return Err(jvm.exception("java/lang/IllegalArgumentException", "Invalid keyboard event type").await);
                };
                let code = event[2];

                let _: () = jvm.invoke_virtual(&display, "handleKeyEvent", "(II)V", (event_type as i32, code)).await?;
            }
            EventQueueEvent::NotifyEvent => {
                let r#type = event[1];
                let param1 = event[2];
                let param2 = event[3];

                let _: () = jvm
                    .invoke_virtual(&display, "handleNotifyEvent", "(III)V", (r#type, param1, param2))
                    .await?;
            }
        }

        Ok(())
    }

    async fn get_event_queue(jvm: &Jvm, _context: &mut WieJvmContext) -> JvmResult<ClassInstanceRef<Self>> {
        tracing::debug!("net.wie.EventQueue::getEventQueue()");

        let event_queue: ClassInstanceRef<Self> = jvm.get_static_field("net/wie/EventQueue", "eventQueue", "Lnet/wie/EventQueue;").await?;
        let event_queue = if event_queue.is_null() {
            let instance = jvm.new_class("net/wie/EventQueue", "()V", ()).await?;
            jvm.put_static_field("net/wie/EventQueue", "eventQueue", "Lnet/wie/EventQueue;", instance.clone())
                .await?;

            instance.into()
        } else {
            event_queue
        };

        Ok(event_queue)
    }

    async fn call_serially(
        jvm: &Jvm,
        _context: &mut WieJvmContext,
        this: ClassInstanceRef<Self>,
        event: ClassInstanceRef<Runnable>,
    ) -> JvmResult<()> {
        tracing::debug!("net.wie.EventQueue::callSerially({this:?}, {event:?})");

        let call_serially_events = jvm.get_field(&this, "callSeriallyEvents", "Ljava/util/Vector;").await?;
        jvm.invoke_virtual(&call_serially_events, "addElement", "(Ljava/lang/Object;)V", [event.into()])
            .await
    }
}
