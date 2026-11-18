"""아바타 옷장 (167차) — the costume sheets as NEW jar members, inside the `patch_asset.py` chain.

`patch_asset.patch_jar` asks [`extra_members`] for them next to the legacy-look palettes; the
members are `res/hero/h<D><CC><49+NN>_bm`, one per (set, sheet group) whose PNG frames the
costume folder carries (`avatar_sheets.members`). Nothing else writes those names, so `--check`
compares them byte for byte, and a costume sheet the shipped jar carries but `costumes/` no
longer makes is stale too ([`is_costume_member`]).

⛔ A member that already exists in the pristine jar is refused (`patch_asset.rebuild` raises) — a
costume never replaces a shipped sheet; sprites 50..99 are free in every group (the shipped
sheets use 0..22).
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import avatar_sheets as AS  # noqa: E402


def extra_members(jar_bytes: bytes = b'') -> dict:
    """`{member: _bm bytes}` for every costume sheet in `costumes/` (the jar is not read: the
    PNGs are the source)."""
    return AS.members()


def is_costume_member(name: str) -> bool:
    """`res/hero/hDCCSS_bm` with sprite 50..99 of one of the twelve groups — the shared rule
    (`wie_util::avatar::parse_sheet_member`)."""
    if not (name.startswith('res/hero/h') and name.endswith('_bm')):
        return False
    stem = name[len('res/hero/h'):-3]
    if len(stem) != 5 or not stem.isdigit():
        return False
    prefix = 'h' + stem[:3]
    sprite = int(stem[3:])
    return any(p == prefix for p, _ in AS.GROUPS.values()) and AS.SHEET_BASE < sprite <= AS.SHEET_BASE + AS.SET_MAX


def verify(after: dict) -> None:
    """Every costume member decodes, frame for frame, to the group's signature."""
    for name, data in after.items():
        frames = AS.decode(data, name)
        prefix = name[len('res/hero/'):len('res/hero/') + 4]
        g = next(g for g, (p, _) in AS.GROUPS.items() if p == prefix)
        if [f.size() for f in frames] != AS.FRAME_SIZES[g]:
            raise SystemExit(f'{name}: frame sizes {[f.size() for f in frames]} are not the group signature')
