"""Rewrite the item-text files inside the shipped game asset, preserving everything else.

  KTF: assets/hero3_fixed.zip -> 0103EFD4.jar -> dat/{i16,i18}_dat
  LGT: assets/hero3_fixed.zip -> 0002865E.jar -> res/dat/{i16,i18}_dat

Both archives are rebuilt entry-by-entry, copying every other member's DECOMPRESSED
bytes and its original compress_type, so nothing but the named files change. The result
is verified by re-reading it out of the rebuilt archives and re-parsing it.

★ 2026-08-14 — TWO members are patched now, not one:
  * `i18_dat` (cat 0x12, consumables) — cash-item renames, `rename_cash_items.py`
  * `i16_dat` (cat 0x10, orbs)        — slot-advisory removal, `strip_orb_slot_text.py`
★ 2026-08-29 — eight more: `i4_dat`..`i11_dat` (`add_reaver_weapons.py`) append the
  `{리버…` drop-only weapons/shield. Count-table patches live in `binary_patches.toml`.
Each has its own `build`/`verify` pair and each is checked independently, so a failure
in one names the file it came from.

★ The member path is LOCATED, not hardcoded (2026-08-11): the two carrier builds ship
the same file under different prefixes (`dat/` vs `res/dat/`), and the jar is named
after the carrier's AID.

★ 2026-08-13 — REBUILT ON THE CORRECT RECORD MODEL, WITH AN `.orig` FALLBACK. The old
pipeline (via `decode_items.py`) shipped an i18_dat whose length prefixes were stale
from record 0x11 on (CASH_SHOP.md §(4)); `rename_cash_items.py` now refuses any input
that does not parse byte-exact, and THIS tool answers the refusal: when the zip's
current i18_dat is the damaged one and `<zip>.orig` exists (the backup this very tool
makes on --apply), the rebuild restarts from the .orig — so one `--apply` run both
repairs the framing and applies the current rename table.

Usage: python patch_asset.py <zip> [--apply | --check]
Without --apply it writes <zip>.new and only reports.
"""
import io
import os
import shutil
import sys
import zipfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import dump_item_db as DB            # noqa: E402  (the correct record model)
import rename_cash_items as RCI      # noqa: E402
import strip_orb_slot_text as ORB    # noqa: E402
import add_reaver_weapons as RV      # noqa: E402
import add_boss_accessories as AC   # noqa: E402
import add_tower_material as TM    # noqa: E402
import add_legacy_boots as LB      # noqa: E402
import add_attack_speed_label as ASL  # noqa: E402
import add_hell_restart_label as HRL   # noqa: E402
import add_regen_label as RGL      # noqa: E402
import add_legacy_items as LI      # noqa: E402
import add_tower_recipes as AR     # noqa: E402
import add_legacy_looks as LK      # noqa: E402
import add_costumes as COS         # noqa: E402  (167차: the 아바타 옷장 costume sheets)

# basename -> the module that rewrites it (each exposes build(bytes) and verify(a, b))
TARGETS = {
    'i18_dat': RCI,   # cat 0x12 consumables — cash-item renames
    'i16_dat': ORB,   # cat 0x10 orbs        — "…에 결합." advisory removal
}
TARGETS.update(RV.HANDLERS)  # i4..i11 — `{리버…` drop-only weapons/shield
TARGETS.update(AC.HANDLERS)  # i12 — `{타이탄의 파편` / `{검은 눈물` boss-only accessories
TARGETS.update(TM.HANDLERS)  # i14 — `{닉스코어의 파편` tower-only material (cat 0x0e)
TARGETS.update(HRL.HANDLERS)  # InGame_txt — THREE appended rows, in this order: ASL's 공격속도
                              # option label at 198 (its row index fixes the stat id:
                              # `label[80 + id]`. ⛔ NOT derived — `add_legacy_boots.
                              # ATTACK_SPEED_STAT` carries the same 0x76 by hand, and
                              # `stat_ids_agree` below holds them together, 135차 §2 ⑥),
                              # then 140차's 「헬모드로 재시작합니다」 at 199, the row the
                              # `ending_label_lgt` hook shows after a hardcore clear —
                              # `hell_label_agrees` holds THAT pair (the hook's constant is
                              # read back out of `endinglabel.rs`). HRL runs ASL inside.
TARGETS.update(RGL.HANDLERS)  # ★ 150차: …then 「5초당 HP회복 +」 at 200 = 80 + 0x78, the label of
                              # 레아의목걸이's host option (`acc_options.REGEN`). RGL runs HRL
                              # inside, so it REPLACES HRL's entry — it must come right after it.
                              # `regen_label_agrees` holds the row against `acc_options.rs`'s
                              # REGEN_LABEL_ROW / REGEN_STAT.
TARGETS.update(LB.HANDLERS)  # i3  — `@섬광의군화`, the first item carrying 공격속도
# ⛔ LAST: it WRAPS whatever i0..i12 handler is registered above (reaver / boss acc /
# boots run first, then the legacy append — the weapon floors must see the reaver rows).
TARGETS.update(LI.handlers(TARGETS))  # i0..i12 — `"` legacy rows = the hell drop table
# ★ 138차: i15 — the 하늘성 조합서 (78 recipes over the network rows 0x27..0x58, + 28). AFTER
# the legacy rows: its `bind` replays the i0..i12 handlers above to find where each result
# lands. The member is ciphertext, so the handler needs the jar's `des_dat` (`bind`).
TARGETS.update(AR.handlers(TARGETS))
# ★ 139차: the LOOK of the 69 drawn `"` rows — a shipped sprite (b4) and a NEW palette (b5)
# each, so the legacy sets stop wearing their templates' sprites. LAST: it wraps the i0..i11
# handlers above and moves only b4/b5 of the `"` rows (names and row order stay, so the
# recipe `bind` above replays it harmlessly). The palettes are NEW jar members
# (`res/dat/ph158_pa`..) — `patch_jar` appends `LK.extra_members`, `check` compares them.
TARGETS.update(LK.handlers(TARGETS))
# The member whose framing decides whether the shipped asset is pristine. `i18_dat` is
# the one the old broken record model damaged, so it is the canary for the .orig fallback.
CANARY_NAME = 'i18_dat'


def find_target(jar_bytes: bytes, base: str) -> str:
    """The one member with this basename, whatever directory it sits in."""
    names = [n for n in zipfile.ZipFile(io.BytesIO(jar_bytes)).namelist()
             if n.rsplit('/', 1)[-1] == base]
    if len(names) != 1:
        raise SystemExit(f'expected exactly one {base} in the jar, got {names}')
    return names[0]


def _entry_like(name: str, info) -> zipfile.ZipInfo:
    """A new ZipInfo for `name`, stored exactly like `info` (some assets are STORED)."""
    zi = zipfile.ZipInfo(name, date_time=info.date_time)
    zi.compress_type = info.compress_type
    zi.external_attr = info.external_attr
    zi.internal_attr = info.internal_attr
    zi.create_system = info.create_system
    return zi


def rebuild(zbytes: bytes, members: dict, extra: dict = None) -> bytes:
    """Rebuild a zip, replacing each `members[name]` through its transform, then appending
    each `extra[name]` (bytes) as a NEW entry stored like the first entry of its folder whose
    name shares its two-letter prefix (a new `ph158_pa` is stored like `ph000_pa`)."""
    src = zipfile.ZipFile(io.BytesIO(zbytes))
    infos = src.infolist()
    out = io.BytesIO()
    seen = set()
    with zipfile.ZipFile(out, 'w') as dst:
        for info in infos:
            data = src.read(info.filename)
            if info.filename in members:
                data = members[info.filename](data)
                seen.add(info.filename)
            dst.writestr(_entry_like(info.filename, info), data)
        have = {i.filename for i in infos}
        for name, data in (extra or {}).items():
            if name in have:
                raise SystemExit(f'{name} is not new — it already exists')
            folder, base = name.rsplit('/', 1) if '/' in name else ('', name)
            kin = sorted((i for i in infos if i.filename.rsplit('/', 1)[0] == folder
                          and i.filename.rsplit('/', 1)[-1][:2] == base[:2]),
                         key=lambda i: i.filename)
            if not kin:
                raise SystemExit(f'{name}: no existing entry in {folder!r} to store it like')
            dst.writestr(_entry_like(name, kin[0]), data)
    missing = set(members) - seen
    if missing:
        raise SystemExit(f'{sorted(missing)} not found')
    return out.getvalue()


def _jar_of(zbytes: bytes) -> zipfile.ZipFile:
    """The game jar inside a zip, opened."""
    z = zipfile.ZipFile(io.BytesIO(zbytes))
    jars = [n for n in z.namelist() if n.lower().endswith('.jar')]
    if len(jars) != 1:
        raise SystemExit(f'expected exactly one jar, got {jars}')
    return zipfile.ZipFile(io.BytesIO(z.read(jars[0])))


def member_of(zbytes: bytes, base: str) -> bytes:
    """The bytes of <zip> -> <jar> -> res?/dat/<base>."""
    z = zipfile.ZipFile(io.BytesIO(zbytes))
    jars = [n for n in z.namelist() if n.lower().endswith('.jar')]
    if len(jars) != 1:
        raise SystemExit(f'expected exactly one jar, got {jars}')
    jb = z.read(jars[0])
    return zipfile.ZipFile(io.BytesIO(jb)).read(find_target(jb, base))


def parses_byte_exact(blob: bytes) -> bool:
    recs = list(DB.records(blob))
    return bool(recs) and recs[-1][1] + 2 + len(recs[-1][2]) == len(blob)


def backup_paths(zpath: str) -> list:
    """Where a pristine copy of `zpath` may live, best first.

    `android_app/asset_backup/` is the documented home for pristine assets (CLAUDE.md)
    and is the ONLY correct one: anything left beside the zip inside `assets/` is packed
    into the APK verbatim, which cost ~1.76 MB of dead weight until 2026-08-14. The
    sibling path stays as a fallback so an older tree still repairs itself.
    """
    name = os.path.basename(zpath) + '.orig'
    here = os.path.dirname(os.path.abspath(zpath))
    # …/android_app/app/src/main/assets -> …/android_app/asset_backup
    android_app = os.path.abspath(os.path.join(here, '..', '..', '..', '..'))
    return [os.path.join(android_app, 'asset_backup', name), os.path.join(here, name)]


def pristine_path(zpath: str):
    """The first existing backup that parses cleanly, or None."""
    for p in backup_paths(zpath):
        if os.path.exists(p) and parses_byte_exact(member_of(open(p, 'rb').read(), CANARY_NAME)):
            return p
    return None


# ── `--check`: the staleness gate this pipeline was missing ──────────────────
#
# ★ 2026-08-26 (71차). Every other generated artefact in this repo has one
# (`gen_cash_catalog.py --check`, `gen_gamefont.py --check`); the ASSET did not, and that
# is precisely the shape of the two worst accidents here. 43차's orb edit was silently
# undone by a later run of this tool and nobody noticed until 44차; 63차 shipped ten
# builds (vc121~vc130) that died at the title screen because the asset and the guest
# patches disagreed. Both were SILENT — the tools printed OK the whole time.
#
# So: rebuild from pristine in memory and byte-compare against the shipped zip. A
# mismatch means the shipped asset no longer reflects the tables in this repo.
#
# ⛔ Only members this tool SOLELY owns can be compared. `i18_dat` is co-owned —
# `patch_item_prices.py` rewrites 41 price fields in it afterwards — so its shipped bytes
# are legitimately different from what `build` alone produces. `i16_dat` is not in that
# tool's plan (cats [0..11, 13, 18]), so it is the one member that must match exactly.
#
# ★ `i14_dat` joined the set when the tower material was added. The price tool only
# rewrites rows that carry the cash marker `}` AND price 0, and i14 has no `}` row at
# all (its one price-0 row, idx 0x1c, is the `(아래부터)` separator), so this member is
# solely ours too. If that ever stops being true the check fails loudly, which is the
# right failure mode — 84차-B is the round where a silently un-reapplied asset shipped.
# ⛔ i3_dat cannot join this list even though `add_legacy_boots` owns a row in it:
# `patch_item_prices.py` rewrites price bytes in the SAME member afterwards, so a
# byte-exact comparison against the tables is guaranteed to differ (the i18_dat
# problem, 93차). The append is still gated by that module's own `verify`.
# ★ `InGame_txt` (135차-B): `add_regen_label` (150차 — which runs `add_hell_restart_label`,
# which runs `add_attack_speed_label`, 140차) is the only tool in the chain that touches it
# (the item tools never open it;
# the price tool only names indices in its doc), so its byte compare is exact — and without
# it a chain that lost the label shipped the 공격속도 option as the game's 「힘 +」 again, with
# every `--check` green.
# ★ `i15_dat` (138차): `add_tower_recipes` is its only writer (the price tool never opens a
# cat-0x0f member, the others are map/script/sound tools) and DES-ECB + MD5 are
# deterministic, so a rebuilt member is byte-identical — a zip that lost the 78 recipes, or
# carries a stale plan, fails here instead of booting a 117-row count over an 89-row table.
CHECK_MEMBERS = ('i16_dat', 'i12_dat', 'i14_dat', 'InGame_txt', 'i15_dat')

# ★★★ 135차 §2 ③ — THE CO-OWNED EQUIPMENT MEMBERS, BY ROW COUNT AND NAME.
#
# i0..i11 carry the reaver rows (75차), the boots row (131차-C) and the 69 legacy rows
# (133차), and the guest's count table (`binary_patches.toml`) was raised to match. A
# member SHORTER than that table is not a cosmetic miss: the allocator reads rows that
# are not there — 63차's `Allocation failure` and 132차 §5's exact signature. But these
# members are co-owned with `patch_item_prices.py` (it rewrites price bytes in i0 and
# i1..i11 afterwards), so they can never join CHECK_MEMBERS, and until now nothing in the
# six-`--check` ritual looked at them at all — only `gen_helldrop_table --check`, which
# is not part of it. Names and the row count are the fields the price tool never
# touches (it writes the four price bytes after the name — `price_span`), so comparing
# them is exact on a correctly built asset and catches every missing or extra row.
NAME_GATED = tuple(f'i{cat}_dat' for cat in range(12))


def record_names(blob: bytes):
    """The raw name bytes (colour escape included) of every record, in order — or `None`
    when the member does not parse byte-exact."""
    if not parses_byte_exact(blob):
        return None
    return [bytes(body[1:1 + body[0]]) for _i, _off, body in DB.records(blob)]


def set_bytes_of(blob: bytes):
    """(b10, b11) of every row of a 20-byte equipment member (i0..i11) — the wear-class label
    the 상세보기 prints (`InGame_txt[0xbb + b10]`) and the game's own 4-piece set id (`0x4f270`,
    armour) — or `None` when the member does not parse byte-exact (150차).

    ★ Exact on a correctly built asset: the price tool writes only the four price bytes right
    after the name (`price_span`), never b10/b11. The legacy armour rows used to carry the
    @팬텀 template's 6/6 — one 「솔티안 전용」 set for all 34 — and nothing gated them."""
    if not parses_byte_exact(blob):
        return None
    return [(body[1 + body[0] + 10], body[1 + body[0] + 11]) for _i, _off, body in DB.records(blob)]


def stat_bytes_of(blob: bytes):
    """Every byte AFTER the price of every row of an equipment member (i0..i11) — sprite,
    palette, level, tier, wear class, native set, options, sub-stats, sockets — or `None`
    when the member does not parse byte-exact (150차 review).

    The looks (b4/b5) and the set bytes (b10/b11) have their own gates with their own
    messages; this one is the net under them: the 150차 rework rewrote option pairs and
    sub-stats in 78 rows and nothing compared those bytes at all. Exact on a correctly built
    asset for the same reason the other two are — the price tool writes the four price bytes
    and nothing else (`patch_item_prices.price_span`)."""
    if not parses_byte_exact(blob):
        return None
    return [bytes(body[1 + body[0] + 4:]) for _i, _off, body in DB.records(blob)]


def stat_ids_agree(ingame_txt: bytes) -> str:
    """`''` when the 공격속도 label's row and the boots' option id say the same id, else the
    reason. The detail screen names an option `label[80 + id]`, so the two MUST move together
    — and each module carries its half by hand."""
    try:
        sid = ASL.stat_id(ingame_txt)
    except SystemExit as e:
        return str(e)  # already names InGame_txt
    if sid != LB.ATTACK_SPEED_STAT:
        return (f'the 공격속도 label sits at id {sid:#04x} but add_legacy_boots.ATTACK_SPEED_STAT is '
                f'{LB.ATTACK_SPEED_STAT:#04x} — the boots would be labelled with another stat')
    return ''


def renamed_names(blob: bytes) -> dict:
    """`{idx: name bytes}` for the records `rename_cash_items` renames, or `{}` if the
    member does not parse. NAMES ONLY — see `check` for why."""
    try:
        out = {}
        for i, _off, body in DB.records(blob):
            if i in RCI.RENAMES:
                out[i] = RCI._fields(body)[0]
        return out
    except Exception:
        return {}


def check(zpath: str, zbytes: bytes, src_bytes: bytes, newzip: bytes) -> None:
    """Fail if the shipped asset's solely-owned members are not what the tables produce."""
    if pristine_path(zpath) is None:
        raise SystemExit(f'{zpath}: --check needs a pristine backup to rebuild from '
                         f'(looked in {", ".join(backup_paths(zpath))})')
    stale = []
    for base in CHECK_MEMBERS:
        want = member_of(newzip, base)
        got = member_of(zbytes, base)
        if want != got:
            stale.append(f'  {base}: shipped {len(got)} B, tables produce {len(want)} B')

    # ★★ i18_dat, BY NAME ONLY (2026-09-14). It cannot join CHECK_MEMBERS — it is
    # co-owned, `patch_item_prices.py` rewrites prices in it afterwards, so a byte compare
    # against what THIS tool produces is guaranteed to fail on a correctly built asset.
    # But leaving it out entirely left a hole with no gate at all, and that hole shipped:
    # this checkout's asset was two builds old (still `오브상자`, no bag cells) and all
    # six --check tools said OK. Names are the one field the price tool never touches, so
    # comparing just those closes the hole without fighting it.
    want_names = renamed_names(member_of(newzip, 'i18_dat'))
    got_names = renamed_names(member_of(zbytes, 'i18_dat'))
    if not want_names:
        stale.append('  i18_dat: the tables produce a file this tool cannot re-read')
    for i, want in sorted(want_names.items()):
        got = got_names.get(i)
        if got != want:
            stale.append(f'  i18_dat record {i:#04x}: shipped {got!r}, tables produce {want!r}')

    # ★★★ i0..i11 by row count and name (135차 §2 ③ — see NAME_GATED).
    for base in NAME_GATED:
        want = record_names(member_of(newzip, base))
        got = record_names(member_of(zbytes, base))
        if want is None:
            stale.append(f'  {base}: the tables produce a file this tool cannot re-read')
        elif got is None:
            stale.append(f'  {base}: the shipped member does not parse byte-exact')
        elif got != want:
            first = next((i for i, (g, w) in enumerate(zip(got, want)) if g != w), min(len(got), len(want)))
            stale.append(f'  {base}: shipped {len(got)} row(s), tables produce {len(want)} — first difference at row {first}')

    # ★ 139차: the legacy LOOKS — b4/b5 of every i0..i11 row (the price tool writes only the
    # four price bytes before them, so this is exact) and the new palettes, byte for byte,
    # with no palette left over from an older plan.
    for base in NAME_GATED:
        try:
            want, got = LK.looks_of(member_of(newzip, base)), LK.looks_of(member_of(zbytes, base))
        except SystemExit as e:
            stale.append(f'  {base}: {e}')
            continue
        if got != want:
            first = next((i for i, (g, w) in enumerate(zip(got, want)) if g != w), min(len(got), len(want)))
            stale.append(f'  {base}: sprite/palette bytes differ from the tables at row {first}')
    # ★ 150차: b10/b11 of every i0..i11 row — the legacy armour's 0 label + native set ids
    # (8..13 on the 24 set pieces, 0 on the legends and 섬광의군화) and everyone else's
    # untouched bytes. Exact for the same reason the looks are (see `set_bytes_of`).
    for base in NAME_GATED:
        want, got = set_bytes_of(member_of(newzip, base)), set_bytes_of(member_of(zbytes, base))
        if want is None or got is None:
            stale.append(f'  {base}: b10/b11 unreadable (the member does not parse byte-exact)')
        elif got != want:
            first = next((i for i, (g, w) in enumerate(zip(got, want)) if g != w), min(len(got), len(want)))
            stale.append(f'  {base}: wear-class/native-set bytes (b10, b11) differ from the tables at row {first}'
                         f' (shipped {got[first] if first < len(got) else None}, tables '
                         f'{want[first] if first < len(want) else None})')
    # ★ 150차 review: and every other non-price byte of those rows (options, sub-stats, ...).
    for base in NAME_GATED:
        want, got = stat_bytes_of(member_of(newzip, base)), stat_bytes_of(member_of(zbytes, base))
        if want is None or got is None:
            stale.append(f'  {base}: stat bytes unreadable (the member does not parse byte-exact)')
        elif got != want:
            first = next((i for i, (g, w) in enumerate(zip(got, want)) if g != w), min(len(got), len(want)))
            if first < len(got) and first < len(want):
                byte = next((b for b, (g, w) in enumerate(zip(got[first], want[first])) if g != w), None)
                stale.append(f'  {base}: stat bytes differ from the tables at row {first}, '
                             f'b{(byte or 0) + 4} (shipped {got[first].hex(" ")}, tables {want[first].hex(" ")})')
            else:
                stale.append(f'  {base}: shipped {len(got)} row(s), tables produce {len(want)}')
    jar_new, jar_got = _jar_of(newzip), _jar_of(zbytes)
    pristine_names = set(_jar_of(src_bytes).namelist())
    added = [n for n in jar_new.namelist() if n not in pristine_names]
    got_names = set(jar_got.namelist())
    for n in added:
        if n not in got_names:
            stale.append(f'  {n}: missing from the shipped jar')
        elif jar_got.read(n) != jar_new.read(n):
            what = 'costume sheet' if COS.is_costume_member(n) else 'palette'
            stale.append(f'  {n}: shipped {what} differs from the tables')
    for n in sorted(got_names - pristine_names - set(added)):
        if n.rsplit('/', 1)[-1].startswith('ph'):
            stale.append(f'  {n}: a palette the tables no longer make')
        elif COS.is_costume_member(n):
            stale.append(f'  {n}: a costume sheet costumes/ no longer makes')

    why = stat_ids_agree(member_of(zbytes, 'InGame_txt'))
    if why:
        stale.append(f'  {why}')
    why = HRL.hell_label_agrees(member_of(zbytes, 'InGame_txt'))
    if why:
        stale.append(f'  {why}')
    why = RGL.regen_label_agrees(member_of(zbytes, 'InGame_txt'))
    if why:
        stale.append(f'  {why}')

    if stale:
        print(f'STALE: {zpath} does not match the tables in this repo')
        print('\n'.join(stale))
        raise SystemExit('re-run with --apply (then patch_item_prices.py --apply)')
    row, stat = RGL.rust_consts()
    print(f'CHECK OK: {zpath} matches the tables ({", ".join(CHECK_MEMBERS)}, i18_dat names, '
          f'i0..i11 rows+names+looks+b10/b11+stat bytes, '
          f'{sum(not COS.is_costume_member(n) for n in added)} look palette(s), '
          f'{sum(COS.is_costume_member(n) for n in added)} costume sheet(s), '
          f'공격속도 id {LB.ATTACK_SPEED_STAT:#04x}, 헬모드 restart row {HRL.EXPECTED_INDEX}, '
          f'5초당 HP회복 row {row} = 80 + {stat:#04x} (acc_options.rs))')


def main() -> None:
    zpath = sys.argv[1]
    apply = '--apply' in sys.argv
    checking = '--check' in sys.argv
    if apply and checking:
        raise SystemExit('--apply and --check are exclusive')
    zbytes = open(zpath, 'rb').read()

    # ── source selection ──
    # The tables must start from a PRISTINE item DB, so a known-good backup ALWAYS wins:
    # rebuilding from the already-patched asset happens to be idempotent for today's
    # tables, but it silently would not be for any table that edits a field it also
    # matches on. Falling back to the shipped zip only covers a tree with no backup yet.
    bak = pristine_path(zpath)
    # ★ 150차 review: say it BEFORE rebuilding — without a pristine source the handlers run
    # over the already-patched asset and die on their own row-count asserts ("expected 49
    # rows, got 57"), which reads like a broken table rather than a missing backup.
    if checking and bak is None:
        raise SystemExit(f'{zpath}: --check needs a pristine backup to rebuild from '
                         f'(looked in {", ".join(backup_paths(zpath))})')
    if bak is not None:
        src_bytes = open(bak, 'rb').read()
        print(f'{zpath}: rebuilding from the pristine backup {bak}')
    elif parses_byte_exact(member_of(zbytes, CANARY_NAME)):
        src_bytes = zbytes
        print(f'{zpath}: no pristine backup found — using the shipped asset as the source')
    else:
        raise SystemExit(
            f'{zpath}: {CANARY_NAME} has the stale-length-prefix damage and no pristine '
            f'backup exists to rebuild from (looked in {", ".join(backup_paths(zpath))}) '
            '— restore a pristine asset first')

    z = zipfile.ZipFile(io.BytesIO(src_bytes))
    jars = [n for n in z.namelist() if n.lower().endswith('.jar')]
    jar = jars[0]
    print(f'{zpath}: {len(z.namelist())} entries, game jar = {jar}')

    def patch_jar(jb: bytes) -> bytes:
        paths = {find_target(jb, base): mod for base, mod in TARGETS.items()}
        for path, mod in paths.items():
            print(f'  item text member = {path}  ({mod.__name__})')
        # A handler that needs more than its own member (138차: the encrypted i15_dat needs
        # `des_dat` and the final i0..i12) is handed the pristine jar first.
        for mod in paths.values():
            if hasattr(mod, 'bind'):
                mod.bind(jb)
        # ★ 139차: the new palettes of the legacy looks, built from this jar's own sprites.
        extras = LK.extra_members(jb)
        # ★ 167차: the 아바타 옷장 costume sheets, packed from `costumes/*/` PNG frames.
        costumes = COS.extra_members(jb)
        COS.verify(costumes)
        extras.update(costumes)
        newjar = rebuild(jb, {p: m.build for p, m in paths.items()}, extras)
        jz_b, jz_a = zipfile.ZipFile(io.BytesIO(jb)), zipfile.ZipFile(io.BytesIO(newjar))
        # Round-trip check per member: byte-exact parse and only the intended rows changed.
        for path, mod in paths.items():
            before, after = jz_b.read(path), jz_a.read(path)
            mod.verify(before, after)
            print(f'  {path}: {len(before)} -> {len(after)} bytes')
        # Every OTHER member of the jar must be byte-identical; the only new ones are ours.
        assert jz_a.namelist() == jz_b.namelist() + list(extras), 'jar member list changed'
        for n in jz_b.namelist():
            if n not in paths and jz_b.read(n) != jz_a.read(n):
                raise SystemExit(f'jar member {n} changed unexpectedly')
        for n, data in extras.items():
            if jz_a.read(n) != data:
                raise SystemExit(f'new jar member {n} did not round-trip')
        names = [n.rsplit('/', 1)[-1] for n in extras if not COS.is_costume_member(n)]
        print(f'  jar: {len(jz_b.namelist())} members, all others byte-identical, '
              f'+ {len(names)} new palette(s) {names[0]}..{names[-1]}'
              f' + {len(costumes)} costume sheet(s)')
        why = stat_ids_agree(jz_a.read(find_target(newjar, 'InGame_txt')))
        if why:
            raise SystemExit(why)
        why = HRL.hell_label_agrees(jz_a.read(find_target(newjar, 'InGame_txt')))
        if why:
            raise SystemExit(why)
        why = RGL.regen_label_agrees(jz_a.read(find_target(newjar, 'InGame_txt')))
        if why:
            raise SystemExit(why)
        return newjar

    newzip = rebuild(src_bytes, {jar: patch_jar})
    z2 = zipfile.ZipFile(io.BytesIO(newzip))
    assert z.namelist() == z2.namelist(), 'zip member list changed'
    for n in z.namelist():
        if n != jar and z.read(n) != z2.read(n):
            raise SystemExit(f'zip member {n} changed unexpectedly')
    print(f'  zip: {len(z2.namelist())} entries, all others byte-identical')

    if checking:
        check(zpath, zbytes, src_bytes, newzip)
        return

    if apply:
        # Only ever back up a PRISTINE asset — backing up an already-patched one would
        # poison the fallback path above. `backup_paths()[0]` is asset_backup/, which is
        # outside `assets/` and so is not packed into the APK.
        if pristine_path(zpath) is None and parses_byte_exact(member_of(zbytes, CANARY_NAME)):
            bak = backup_paths(zpath)[0]
            os.makedirs(os.path.dirname(bak), exist_ok=True)
            shutil.copy2(zpath, bak)
            print(f'  backup -> {bak}')
        open(zpath, 'wb').write(newzip)
        print(f'  APPLIED: {zpath}  {len(zbytes)} -> {len(newzip)} bytes')
    else:
        # NOT beside the zip: `assets/` is packed verbatim into the APK.
        out = os.path.join(os.path.dirname(backup_paths(zpath)[0]),
                           os.path.basename(zpath) + '.new')
        os.makedirs(os.path.dirname(out), exist_ok=True)
        open(out, 'wb').write(newzip)
        print(f'  dry run -> {out}  {len(zbytes)} -> {len(newzip)} bytes')


if __name__ == '__main__':
    main()
