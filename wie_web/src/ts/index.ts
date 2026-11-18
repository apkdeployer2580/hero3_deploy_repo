import { WieWeb } from "@pkg";
import { setMasterVolume } from "./midi";

const TUTORIAL_STORAGE_KEY = "wie_tutorial_dismissed";

const initTutorial = () => {
  const overlay = document.getElementById("tutorial-overlay");
  const closeButton = document.getElementById("close-tutorial");
  const dontShowCheckbox = document.getElementById("dont-show-again") as HTMLInputElement;

  if (!overlay || !closeButton || !dontShowCheckbox) return;

  // The intro popup no longer auto-opens on launch (user preference). It stays
  // available on demand via the ❓ button in the corner.

  const closeTutorial = () => {
    if (dontShowCheckbox.checked) {
      localStorage.setItem(TUTORIAL_STORAGE_KEY, "true");
    } else {
      localStorage.removeItem(TUTORIAL_STORAGE_KEY);
    }
    overlay.classList.remove("visible");
  };

  closeButton.addEventListener("click", closeTutorial);

  overlay.addEventListener("click", (e) => {
    if (e.target === overlay) {
      closeTutorial();
    }
  });

  const showTutorialButton = document.getElementById("show-tutorial");
  showTutorialButton?.addEventListener("click", () => {
    overlay.classList.add("visible");
  });
};

const key_map: Record<string, string> = {
  Digit1: "1",
  Digit2: "2",
  Digit3: "3",
  KeyQ: "4",
  KeyW: "5",
  KeyE: "6",
  KeyA: "7",
  KeyS: "8",
  KeyD: "9",
  KeyZ: "*",
  KeyX: "0",
  KeyC: "#",
  Backspace: "CLR",
  ArrowUp: "UP",
  ArrowLeft: "LEFT",
  ArrowRight: "RIGHT",
  ArrowDown: "DOWN",
  Space: "OK",
};

const LAYOUT_STORAGE_KEY = "wie_control_layout";

const initControls = () => {
  const controls = document.getElementById("controls");
  const select = document.getElementById("layout-select") as HTMLSelectElement | null;
  if (!controls) return;

  const saved = localStorage.getItem(LAYOUT_STORAGE_KEY) ?? "joystick";
  controls.dataset.layout = saved;
  if (select) {
    select.value = saved;
    select.addEventListener("change", () => {
      controls.dataset.layout = select.value;
      localStorage.setItem(LAYOUT_STORAGE_KEY, select.value);
    });
  }
};

// Floating joystick: touch ANYWHERE in the movement zone -> the joystick spawns
// at the touch point; drag relative to it -> directional KeyCodes. This avoids
// the "finger drifted to center, must return to a fixed pad" problem.
const setupJoystick = (wie: WieWeb) => {
  const zone = document.getElementById("move-zone");
  const js = document.getElementById("joystick");
  const knob = js?.querySelector(".joystick-knob") as HTMLElement | null;
  if (!zone || !js || !knob) return;

  const MAXR = 60; // knob max throw (px)
  const DEAD = 16; // deadzone before a direction first registers (px)
  const DEAD_RELEASE = 8; // once engaged, stay engaged until the finger returns this close to center (hysteresis)
  const ENGAGE = 0.383; // sin(22.5deg): threshold to ADD a direction (8-way feel; diagonals press two keys)
  const KEEP = 0.18; // looser threshold to KEEP an already-held direction (hysteresis)
  let active = new Set<string>();
  let startX = 0;
  let startY = 0;
  let pid = -1;

  const setDirs = (next: Set<string>) => {
    for (const d of active) if (!next.has(d)) wie.key_up(d);
    for (const d of next) if (!active.has(d)) wie.key_down(d);
    active = next;
  };

  const begin = (e: PointerEvent) => {
    // Already tracking a finger: ignore extra touches in the move-zone so a
    // second touch can't hijack the pointer id and orphan the held direction.
    if (pid !== -1) return;
    pid = e.pointerId;
    startX = e.clientX;
    startY = e.clientY;
    const zr = zone.getBoundingClientRect();
    js.style.left = `${e.clientX - zr.left}px`;
    js.style.top = `${e.clientY - zr.top}px`;
    js.classList.add("active");
    knob.style.transform = "translate(0px, 0px)";
    try {
      zone.setPointerCapture(e.pointerId);
    } catch {
      /* ignore */
    }
    e.preventDefault();
  };

  const move = (e: PointerEvent) => {
    if (e.pointerId !== pid) return;
    const dx = e.clientX - startX;
    const dy = e.clientY - startY;
    const dist = Math.hypot(dx, dy) || 1;
    const cl = Math.min(dist, MAXR);
    knob.style.transform = `translate(${(dx / dist) * cl}px, ${(dy / dist) * cl}px)`;

    // Hysteresis: a direction engages at ENGAGE but only releases below KEEP,
    // and once any direction is engaged the stick only fully re-centers below
    // DEAD_RELEASE. This stops a transient/coalesced pointermove (common while
    // the main thread is busy emulating a heavy scene) from briefly dropping a
    // held direction -> spurious key_up -> the character freezing while the
    // finger is still pushed that way.
    const engaged = active.size > 0;
    const dead = engaged ? DEAD_RELEASE : DEAD;
    const next = new Set<string>();
    if (dist > dead) {
      const nx = dx / dist;
      const ny = dy / dist;
      const th = (d: string) => (active.has(d) ? KEEP : ENGAGE);
      if (ny < -th("UP")) next.add("UP");
      if (ny > th("DOWN")) next.add("DOWN");
      if (nx < -th("LEFT")) next.add("LEFT");
      if (nx > th("RIGHT")) next.add("RIGHT");
    }
    setDirs(next);
    e.preventDefault();
  };

  const end = (e: PointerEvent) => {
    if (e.pointerId !== pid && pid !== -1) return;
    pid = -1;
    js.classList.remove("active");
    setDirs(new Set());
  };

  zone.addEventListener("pointerdown", begin);
  zone.addEventListener("pointermove", move);
  zone.addEventListener("pointerup", end);
  zone.addEventListener("pointercancel", end);
};

// Fullscreen + landscape-lock (best effort; iOS ignores lock -> user rotates).
const initFullscreen = () => {
  const btn = document.getElementById("fullscreen-toggle");
  btn?.addEventListener("click", async () => {
    try {
      if (!document.fullscreenElement) {
        await document.documentElement.requestFullscreen();
      }
      const orientation = (screen as unknown as { orientation?: { lock?: (o: string) => Promise<void> } }).orientation;
      if (orientation?.lock) {
        await orientation.lock("landscape").catch(() => {});
      }
    } catch {
      /* fullscreen/orientation not supported -> user rotates the phone */
    }
  });
};

const main = () => {
  initVolume();

  const file = document.getElementById("file") as HTMLInputElement;
  const fileName = document.getElementById("file-name");

  file.addEventListener("change", () => {
    const selected_file = file.files?.[0];
    if (fileName) {
      fileName.textContent = selected_file?.name ?? "파일 선택";
    }

    if (selected_file) {
      const prestart = document.getElementById("prestart");
      if (prestart) {
        // only remove the file-picker screen; keep corner-actions (⚙️ 설정 / ⛶ 전체화면)
        // accessible over the canvas during play.
        prestart.classList.add("hidden");
        setTimeout(() => {
          prestart.remove();
        }, 300);
      }

      // NB: we intentionally do NOT override console.error to alert(). Doing so
      // turned every third-party console.error (notably AdSense's
      // "adsbygoogle.push() … availableWidth=0" on rotation) into a blocking
      // popup. Genuine emulator failures are already surfaced by the try/catch
      // blocks around construction and the update loop below.
      const reader = new FileReader();

      reader.onload = (e) => {
        const data = e.target?.result as ArrayBuffer;

        try {
          const canvas = document.getElementById("canvas") as HTMLCanvasElement;
          const wie_web = new WieWeb(
            selected_file.name,
            new Uint8Array(data),
            canvas
          );
          const pcmSlider = document.getElementById("volume-pcm") as HTMLInputElement;
          wie_web.set_pcm_volume(Number(pcmSlider.value) / 100);
          pcmSlider?.addEventListener("input", () => wie_web.set_pcm_volume(Number(pcmSlider.value) / 100));

          for (const button of document.querySelectorAll("button[data-key]")) {
            const handleKeyDown = (e: Event) => {
              e.preventDefault();
              const key = (e.currentTarget as HTMLButtonElement).dataset.key;
              if (key) wie_web.key_down(key);
            };
            const handleKeyUp = (e: Event) => {
              e.preventDefault();
              const key = (e.currentTarget as HTMLButtonElement).dataset.key;
              if (key) wie_web.key_up(key);
            };

            button.addEventListener("mousedown", handleKeyDown);
            button.addEventListener("mouseup", handleKeyUp);
            button.addEventListener("touchstart", handleKeyDown);
            button.addEventListener("touchend", handleKeyUp);
          }
          setupJoystick(wie_web);
          document.addEventListener("keydown", (e) => {
            if (key_map[e.code]) {
              e.preventDefault();
              wie_web.key_down(key_map[e.code]);
            }
          });
          document.addEventListener("keyup", (e) => {
            if (key_map[e.code]) {
              e.preventDefault();
              wie_web.key_up(key_map[e.code]);
            }
          });
          const update = () => {
            try {
              wie_web.update();
            } catch (e) {
              wie_web.free();
              alert(String(e));
              throw e;
            }

            requestAnimationFrame(update);
          };

          requestAnimationFrame(update);
        } catch (e) {
          alert(String(e));
          throw e;
        }
      };

      reader.readAsArrayBuffer(selected_file);
    }
  });
};

const initVolume = () => {
  const toggle = document.getElementById("settings-toggle");
  const panel = document.getElementById("settings-panel");
  const midiSlider = document.getElementById("volume-midi") as HTMLInputElement;

  toggle?.addEventListener("click", () => panel?.classList.toggle("visible"));
  document.addEventListener("click", (e) => {
    if (!toggle?.contains(e.target as Node) && !panel?.contains(e.target as Node)) {
      panel?.classList.remove("visible");
    }
  });

  setMasterVolume(Number(midiSlider.value) / 100);
  midiSlider?.addEventListener("input", () => setMasterVolume(Number(midiSlider.value) / 100));
};

if (document.readyState !== "loading") {
  initTutorial();
  initControls();
  initFullscreen();
  main();
} else {
  document.addEventListener("DOMContentLoaded", () => {
    initTutorial();
    initControls();
    initFullscreen();
    main();
  });
}

// Register the offline service worker (installable PWA / "add to home screen").
if ("serviceWorker" in navigator) {
  window.addEventListener("load", () => {
    navigator.serviceWorker.register("/sw.js").catch(() => {
      /* offline support unavailable — non-fatal */
    });
  });
}
