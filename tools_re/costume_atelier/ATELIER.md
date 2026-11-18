# Costume atelier — drawing an avatar set pixel by pixel

This folder draws every avatar costume set (아바타 옷장, `AVATAR_WARDROBE_PLAN.md`) as **new pixel
art** — never a recolour of the game's shipped sheets. A set is one Python file, `sets/<slug>.py`,
that writes every frame from text grids; `build.py` turns it into `costumes/NN_<slug>/*.png` + a
`costume.txt`, and the asset chain packs those into the game.

**Worked example: `sets/red_fox.py`** (set 08, Ritz in a crimson long coat). Read it top to bottom
before drawing anything — every technique below is used there.

## 1. The quality bar

* The user asked for sets that look like their reference art and for pixels that are *pretty*.
  "A shipped sprite with new colours" is exactly what they rejected. Draw shapes: a coat's tails, a
  fur collar, a scarf, a mask, a weapon's real silhouette (a balloon sword must look like a twisted
  balloon, not a sword painted pink).
* Judge at **game size**. The game shows a sprite 1:1 on a phone; `preview.py` writes
  `<set>_game.png` at 3x. A detail that only reads at 8x does not exist. Big readable shapes, clear
  value contrast between parts, a consistent light from the top-left.
* Pixel-art craft: 4–6 tone ramps per material with hue shift (shadows cooler/more saturated,
  highlights warmer), clean clusters (no lone noise pixels), a dark outline round the silhouette but
  softer internal lines, highlights used sparingly, no pillow shading. Keep each material's ramp
  distinct from the hero's skin and from neighbouring materials.

## 2. Tools

```
python tools_re/costume_atelier/preview.py <slug> --out <dir>    # frames / poses / zoom / game / helmet PNGs
python tools_re/costume_atelier/preview.py --zones --out <dir>   # the leg/arm templates' zone maps
python tools_re/costume_atelier/helmguide.py ritz|kei            # where the helmet sits on each head
python tools_re/costume_atelier/build.py --lint <slug>           # sizes, colours, hair coverage, names
python tools_re/costume_atelier/build.py --out <dir> <slug>      # write the PNG set somewhere to inspect
```

Look at the PNGs with an image viewer (agents: the Read tool shows them). **Iterate**: draw, preview,
look at `_zoom.png` and `_game.png`, fix, again. Never write `costumes/` yourself (`--force` is the
maintainer's step).

| module | what it is |
|---|---|
| `engine.py` | `Mat` (a material: a 6-tone ramp + how it shades), text grids → shaded RGBA |
| `kit.py` | helpers: `family`, `legs`, `arms`, `frames`, `straight`, `weapon`, `hair_pal`, `circle`, `flip`, `cuff` |
| `bodies.py` | the LEG (18) and ARM (12) templates: zones traced from the shipped sheets + their form shading |
| `garments.py` | shared overlays: `LONG_COAT` (long coat tails for all 18 leg frames) + `coat_keys` |
| `rig.py` | weapons: one straight drawing → every turned frame (fitted to the shipped placements) |
| `circles.py` | the dark/holy stone magic circle (66x45) |
| `look.py`, `preview.py`, `helmguide.py` | the game's paper doll offline: put a set on a hero and look |

## 3. What the game draws (the paper doll)

A hero is assembled per pose from layers; each layer frame is placed by its **frame number and
size**, so a costume must keep every size exactly (`avatar_sheets.FRAME_SIZES`, the lint checks it).

| part | frames | what they are |
|---|---|---|
| `helmet` | 4 | **j0 back** of the head (21x28) · j1 rare 3/4 (23x28) · **j2 side, faces LEFT** (21x28) · **j3 front** (18x26). Drawn OVER the head. |
| `armor` | 3 | the torso: **0 back** (12x10) · **1 front** (12x10) · **2 side, faces left** (11x11). The top rows hide under the head; the chest that shows is about rows 3–8; shoulders peek out at the sides. |
| `gloves` | 12 | one arm (sleeve + hand) in the poses the game needs. Zones: `U` upper sleeve · `L` lower sleeve · `C` cuff · `H` hand. |
| `shoes` | 18 | the legs + hips (trousers, boots, coat tails, skirts). Zones: `W` hip · near leg `T` thigh `U` knee `S` shin `A` ankle `F` foot · far leg lowercase. Frames: 0–3 back (**1 = back idle**) · 4 sit · 5 front narrow · **6 front wide (the usual front)** · 7–8 front walk · 9 front crouch · 10 jump · 11 side narrow · **12 side idle (the usual side)** · 13 lunge · 14 wide step · 15–16 side walk · 17 side crouch. **Side frames 11–17 face LEFT** (the game flips them for the right). The top rows of `W` hide under the torso. |
| `weapon_<kind>` | 6/5/4/6/7/1/1 | spear · sword · knife (claws/knuckles) · gun · rifle · dark · holy |
| `shield` | 1 | 20x14 (shown when the main weapon is a spear) |

Which weapons a set needs: **Kei** sets → spear, sword, knife, dark, holy. **Ritz** sets → spear,
sword, knife, gun, rifle. Sets for both heroes → all seven.

**Helmet = hair.** A helmet frame is drawn over the hero's head, so a hairstyle/hat/hood helmet must
**cover every hair pixel** of the hero's own head (else his own hair peeks out) and leave the face
visible. `helmguide.py <hero>` prints each frame with `h` = hair to cover and `f` = face; the lint
fails on uncovered hair. A mask may be drawn ON the face (`f` pixels) — e.g. a visor over the eyes.
A set whose helmet is only sunglasses/a mask (the hero's own hair stays) sets
`HELMET_COVERS_HAIR = False`. Sets for both heroes must cover both heads.

**★ 171차 — the head is made smaller AFTER you draw it (`helmfit.py`, applied by `build.py` and
`preview.py` to every set).** Draw the helmet exactly as above (covering the hero's head). Then:

* a set whose TEXT says **`head = own`** gets the hero's own head composited UNDER the drawing (its
  face, pixel for pixel), and two columns (one each side of the face) and two rows above the face
  carved out — the seams whose neighbours match best — so the head comes out ~2 px narrower and
  shorter than the hero's own head while spikes, horns, brims and the face keep their shapes. The
  game then leaves the hero's own head out while this 투구 is shown (painter hook
  `handle_head_paint_lgt`). A set for BOTH heroes can be `head = own` only when its drawing hides
  both heads completely (a mask — there is one sheet and the faces differ); the lint says so.
  `HELMET_SHRINK = (cols, rows)` changes the carve (`None` = composite only);
* any other hair helmet (이안 — face visible, both heroes) keeps the hero's head under it and gets
  `tighten`: the overhang beyond the head is halved (pixels removed from its middle, outline kept);
  `HELMET_FIT = (keep_side, keep_top, margin)` changes it, `None` opts out;
* sunglasses-only helmets (`HELMET_COVERS_HAIR = False`) are untouched.

`python tools_re/costume_atelier/helmfit.py` prints what each set's head becomes.

## 4. The text-grid language

A frame is a block of text, one character per pixel. `.` or space = transparent; any other
character is a key into the set's palette (`dict key -> Mat`).

```python
COAT = '1e0306 4a0810 6e1018 961c22 c0342c e2634a'   # tones 0 outline · 1 deep · 2 shadow · 3 mid · 4 light · 5 highlight
PAL = {'P': Mat(BLACK, z=2), 'B': Mat(LEATHER, z=3, gloss=True), 'Y': Mat(GOLD, z=4, auto=True)}
```

`Mat(ramp, z=0, auto=False, flat=None, noline=False, gloss=False, line=0, edge=None, shift=0, fam=None, style='plain')`

* automatic shading: the outer silhouette takes tone `line` (0); a part outlines its edge against a
  lower-`z` part (tone `edge`); a higher-`z` part above/left casts a shadow (≤2); inside, `style`
  decides: `plain` (lit top/left edge 4, dark bottom/right 2, else 3), `cloth` (cylinder), `metal`.
* `flat=t` pins every pixel of the material to tone t — **this is how you hand-shade**: give each
  tone its own key with `kit.family('012345', RAMP, z, fam)` (or `kit.family('LM', RAMP, tones=(1, 3))`)
  and paint the tones yourself. Hand shading looks better than auto shading for anything that
  matters (torso, hair, weapons, coat tails). Keys of one `fam` shade as one region.
* a `|` after a grid row starts a TONE grid: digits pin that pixel's tone (`'PPPP | 1233'`).
* body templates (`kit.legs`/`kit.arms`) carry the shipped sheet's form shading as tones, so mapped
  trousers keep their knees and folds; `auto=True` ignores that.

### Building each part

```python
parts['armor']  = kit.frames([BACK, FRONT, SIDE], TORSO, 'armor')        # hand-drawn frames at fixed sizes
parts['gloves'] = kit.arms({'U': 'R', 'L': 'R', 'C': 'Y', 'H': 'K'}, PAL, overlays=[...])
parts['shoes']  = kit.legs(LEGMAP, PAL, overlays=[garments.LONG_COAT, FLAMES], under=garments.COAT_UNDER, post=fn)
parts['helmet'] = kit.frames([H0, H1, H2, H3], kit.hair_pal(HAIR), 'helmet')
parts['shield'] = kit.frames([SHIELD], WPAL, 'shield')
parts['weapon_dark'] = kit.circle(PRIMS, circles.ramp('9050e0'))
```

* `kit.legs(mapping, pal, overlays, under, post)` dresses every leg frame: `mapping` zone → key
  (a zone mapped to `''` vanishes), then each overlay. An overlay is a block, or a dict
  `{frame: block}` (frames without an entry get none). In an overlay `~` keeps what is below and
  `_` erases. Keys listed in `under` only paint where nothing is (a garment's back panel BEHIND the
  legs). `post(grid, frame)` edits the key grid before shading (e.g. `kit.cuff` = a boot's top rim).
* `kit.arms` is the same for the 12 arm frames.
* Overlays are drawn at the frame's own size (top-left aligned); see `garments.LONG_COAT` for a full
  18-frame example and `red_fox.FLAMES` for sparse per-frame decoration.
* Coats: `garments.LONG_COAT` + `garments.coat_keys(coat, lining, hem)` give crimson-coat-style
  tails in every frame (front panels, the back panel behind the legs, lining, a hem band). Use it or
  draw your own per-frame overlay dict the same way (wide trousers, a skirt, a fur cloak, a sash).

### Weapons

Each kind is drawn **once, straight**, on the kind's straight canvas; `kit.weapon` turns it into the
other frames (fitted to where the game puts the hand). Orientation of the straight drawing:

| kind | canvas | layout of the straight drawing | hand-drawn frames |
|---|---|---|---|
| sword | 48x8 | pommel at the LEFT (x≈1–3), grip, guard ≈x7–12, blade to the RIGHT tip ≈x46 | — |
| spear | 44x9 head + 20x3 shaft | the head's TIP at the LEFT (x0, row 4), the shaft running right; pass the 20x3 shaft drawing as `hand={'shaft': …}` | — |
| knife | 34x21 | gauntlet/knuckle at the upper RIGHT, claws/blades pointing to the lower LEFT | — |
| gun | 26x15 | barrel pointing RIGHT, grip down-left | 2 = 11x24 pointing DOWN (muzzle at the bottom) · 4 = 10x22 pointing UP |
| rifle | 32x13 | muzzle at the LEFT, stock at the RIGHT | 0 = 9x28 muzzle DOWN · 1 = 10x30 muzzle UP · 4 = 15x24 slung on the back (strap) · 5 = 11x3 shot/flash · 6 = 7x7 diagonal shot |
| dark, holy | 66x45 | a magic circle under the caster (`circles.py`) | — |

```python
out['weapon_sword'] = kit.weapon('sword', kit.straight(SWORD, WPAL, 'sword'))
out['weapon_spear'] = kit.weapon('spear', kit.straight(SPEAR, WPAL, 'spear'), hand={'shaft': kit.straight(SHAFT, WPAL, 'shaft')})
out['weapon_gun']   = kit.weapon('gun', kit.straight(GUN, WPAL, 'gun'), hand={2: kit.paint(DOWN, WPAL, (11, 24)), 4: kit.paint(UP, WPAL, (10, 22))})
```

Turned frames are resampled, so draw weapons with **solid 2–3 px wide shapes and a 1 px dark
outline** — 1 px details break when turned. Look at every turned frame in `_frames.png`.

### Magic circles (dark / holy)

`circles.render(prims, pal)`: a dark silhouette (ellipse + optional `blob`/`lobe` shapes) with glowing
one-pixel strokes (`ring`, `poly` star, `ticks`, `arc`, `seg`, `dots`, centre `stamp`). Stroke weight
1 = line, 2 = bright, 3 = bold. `circles.ramp('hex')` builds a palette from one line colour. Give the
set's two circles different designs (dark = the set's shadowy colour, holy = its light colour) that
echo the set's emblem; look at the shipped ones (`h509NN`/`h510NN`) for the style.

## 5. Workflow for one set

1. Decide the design from the brief: silhouette, 3–6 materials, an emblem/motif, the weapon theme.
2. Palettes: one 6-tone ramp per material (hex, tones 0..5).
3. Torso (`armor` 0 back / 1 front / 2 side) by hand with `kit.family` keys.
4. Legs: map zones, then overlays for anything that changes the silhouette (coat tails, wide
   trousers, skirts, cloaks, belts with pouches, boot cuffs) — per frame. Look at all 18 frames.
5. Arms: map zones, overlays for cuffs/bracers/epaulets/fur.
6. Helmet: run `helmguide.py <hero>`, draw the four frames covering every `h`, keep the face.
   See §7 — hair is the most visible part of these big-headed sprites; spend time on it.
   A hat/hood/mask sits on top of hair you also draw (whatever hair would still show).
7. Weapons straight drawings + hand frames; shield; the two circles (Kei/both-hero sets).
8. `preview.py`, look, fix — repeatedly. Then `build.py --lint <slug>` must say OK.
9. `TEXT` (costume.txt): names ≤ 96 px, set name and descriptions ≤ 143 px (Hangul 11 · space 5 ·
   `.`/`,` 4 · other ASCII 8), only KS X 1001 Hangul + ASCII. The lint checks.

## 6. A set module

```python
NUM = '09'; FOLDER = '09_imperial_guard'; HERO = 'kei'      # None = both heroes
TEXT = """set = …\nacquire = starter\nhero = kei\nhelmet = …\n…"""
def build(): return {part: [frames]}
```

Register the slug in `build.py`'s `SETS` list (the maintainer does this). Keep everything a set needs
inside its own file (or a `sets/_shared_<topic>.py` used only by sets you own); do not edit the shared
modules — ask for a change instead.

## 7. Hair (and anything on the head)

⛔ **Which head is whose** (checked on the device, 169차): the hero whose class name (`CHAR+0xb8`) is **리츠 = Ritz wears the SILVER head `res/hero/h00000`**; **케이 = Kei wears the ORANGE spiky head `h00001`**. 168차 assumed the opposite and every hero-only helmet had to be re-fitted — `helmguide.HEAD_SHEET` / `look.HEADS` are the one place this is written; never hard-code it elsewhere.

The game's own heads (`helmguide.head('ritz')`, look at them at 10x) show the style to match:
**no black outline** — the hair's darkest tone is its edge; soft clusters of 3–4 tones; a highlight
band across the crown on the upper left; a ragged silhouette of tapered locks; fringe locks pointing
down over the forehead between the eyes. Hand-tone hair with digit keys (`kit.hair_pal(ramp)`:
`'0'..'5'` are flat tones — the shipped look; `'h'` auto-shades a region, `'1'` separates locks):

* start from the guide: every `h`/`o` pixel of `helmguide.py <hero>` must become opaque; add
  1–3 px spikes/tips outside it for the silhouette (never a solid round cap);
* tone 0 only on the outer edge (the ramp's tone 0 is a dark shade of the hair colour — near-black
  only for black hair); lock separations in tone 1–2; light from the top-left;
* leave every `f` (face) pixel transparent unless the design puts something on the face (a visor,
  a mask, a scarf over the mouth — draw it in its own material);
* the helmet frame ends at the chin: **long hair that falls past the head** (a ponytail, hair down
  the back) continues on the **armor** frames (0 back: down the middle of the back · 2 side: behind,
  on the right) — the torso is drawn before the head, so it shows below it. A ponytail tied at the
  back of the head is drawn in helmet j0/j2 down to the frame's bottom, then on the torso;
* `sets/red_fox.py` HELM0..HELM3 is a worked example (black messy hair, ~1 hour of hand-toning).

Frame use (cif records): back idle = shoes 1 · armor 0 · head 0 · helmet 0; front idle = shoes 6 ·
armor 1 · head 1 · helmet **3**; side idle = shoes 12 · armor 2 · head 2 · helmet 2. Layers are drawn
legs → torso → arms → head → helmet → weapon (the far arm first in front views).
