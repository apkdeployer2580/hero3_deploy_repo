"""캐시 아이템을 상인에게 팔 수 있게 한다 — 아이템DB 의 가격 4바이트만 고친다.

왜 이게 필요한가 (RE 근거, 2026-08-14):
상점의 판매 거부 "아이템을 팔 수 없습니다"(InGame_txt 0x9a) 는 등급이 아니라 **가격 0**
게이트다. 공용 메시지박스 `0x46b7c` 가 code 0x0b 일 때만 그 문자열을 내고, 그 code 로
들어오는 호출자는 상점 스테이트머신(`0x1e060`) state 8 의 `0x1e36a` 하나뿐이다.
거기 도달하는 분기가 `0x1e364 cmp r3,#0 / 0x1e366 bne` 이고 r3 = 판매가다.

★ 판매가 = **price >> 2** 다 (`0x22e6e ldr r2,[r3,#8]; asrs r2,r2,#2`). 그래서 price=1 은
소용이 없고(1>>2 = 0 이라 여전히 거부), **최소 4** 여야 1골드에 팔린다.

게스트 분기(`0x1e366` 하프워드 하나)를 죽이는 길도 있지만 **택하지 않는다**: 그 분기는
가격 0 인 *모든* 아이템이 공유하고 거기엔 퀘스트템이 들어 있다. 즉 가격 0 은 지금
"팔면 안 되는 것들"의 유일한 보호막이다. 그래서 캐시템만 골라 가격을 올린다.

레코드 모델은 `dump_item_db.py` 의 것을 그대로 재사용한다(`[u16 reclen][u8 namelen][name]
[4B price]…`). 가격은 장비/비장비 모두 이름 바로 뒤 4바이트라 레코드 길이가 변하지 않는다
— 프레이밍이 절대 흔들리지 않는다는 뜻이고, 33차의 그 사고가 재발할 수 없는 형태다.

usage:
    python tools_re/patch_item_prices.py <asset zip>            # dry run -> <zip>.new
    python tools_re/patch_item_prices.py <asset zip> --apply
"""
import importlib.util
import io
import os
import shutil
import struct
import sys
import zipfile

_spec = importlib.util.spec_from_file_location(
    "dump_item_db", os.path.join(os.path.dirname(os.path.abspath(__file__)), "dump_item_db.py"))
DB = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(DB)

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import patch_asset as PA  # noqa: E402  (only for `backup_paths` — the ONE place that
#                            knows where a copy of the asset may live)


def _out_dir(zpath: str) -> str:
    """Where this tool's own by-products go: `android_app/asset_backup/`, never `assets/`.

    ★ 71차-B. Anything left beside the zip inside `assets/` is packed into the APK
    verbatim — that is exactly how vc166 carried 3.53 MB of dead backups (§71차). This
    tool used to drop `<zip>.preprice` and `<zip>.new` right there, so every run since
    2026-08-14 re-created the bloat and the fix was "remember to move it by hand".
    `patch_asset.backup_paths()[0]` is the single source of truth for that directory.
    """
    return os.path.dirname(PA.backup_paths(zpath)[0])


SELL_PRICE_SHIFT = 2          # 판매가 = price >> 2  (guest 0x22e6e)
TARGET_PRICE = 1 << SELL_PRICE_SHIFT   # = 4 -> 판매가 1골드

# 캐시 마커. 이름 첫 바이트가 텍스트 색상 코드이고 `}` 금색이 캐시템이다 (§2.21).
CASH_MARKER = "}"

# ★ 가격 0 을 그대로 둬야 하는 것들 — 팔리면 퀘스트가 깨진다.
#   i17 은 전 행이 퀘스트템이고, i18 0x09 그리폰의피리도 퀘스트템이다.
#   둘 다 캐시 마커가 없으므로 아래 선별 규칙(마커 `}`)에 애초에 걸리지 않지만,
#   규칙이 나중에 느슨해져도 사고가 안 나도록 명시적으로 한 번 더 막는다.
PROTECTED = {(0x11, None), (0x12, 0x09)}


def is_protected(cat, idx):
    return (cat, None) in PROTECTED or (cat, idx) in PROTECTED


def price_span(body):
    """레코드 body 안에서 가격 4바이트가 놓인 (시작, 끝). 장비·비장비 공통이다."""
    namelen = body[0]
    start = 1 + namelen
    return start, start + 4


def plan(dats):
    """[(cat, idx, name, file_offset, old_price)] — 실제로 고칠 자리."""
    out = []
    for cat, blob in sorted(dats.items()):
        for idx, off, body in DB.records(blob):
            if is_protected(cat, idx):
                continue
            nm = body[1:1 + body[0]]
            marker = DB.COLOUR.get(nm[0], ("", ""))[0] if nm else ""
            if marker != CASH_MARKER:
                continue
            s, e = price_span(body)
            old = struct.unpack("<I", body[s:e])[0]
            if old != 0:
                continue                      # 이미 팔리는 캐시템은 건드리지 않는다
            name = DB.euc(nm[1:])
            out.append((cat, idx, name, off + 2 + s, old))
    return out


def patch_blob(cat, blob, wanted):
    """그 카테고리의 dat 을 고쳐서 돌려준다. 길이는 반드시 같다."""
    b = bytearray(blob)
    for (c, idx, name, fo, old) in wanted:
        if c != cat:
            continue
        assert b[fo:fo + 4] == b"\0\0\0\0", f"cat {cat} idx {idx}: 가격이 0 이 아니다"
        b[fo:fo + 4] = struct.pack("<I", TARGET_PRICE)
    assert len(b) == len(blob)
    return bytes(b)


def verify(cat, before, after, wanted):
    """(1) 바이트 정확 파싱 (2) 레코드 경계 불변 (3) 의도한 4바이트만 변경."""
    rb = list(DB.records(before))
    ra = list(DB.records(after))
    assert len(rb) == len(ra), f"i{cat}_dat: 레코드 수가 변했다 {len(rb)} -> {len(ra)}"
    assert rb and rb[-1][1] + 2 + len(rb[-1][2]) == len(before), f"i{cat}_dat: 원본이 클린 파싱이 아니다"
    assert ra[-1][1] + 2 + len(ra[-1][2]) == len(after), f"i{cat}_dat: 결과가 클린 파싱이 아니다"
    for (ib, ob, bb), (ia, oa, ba) in zip(rb, ra):
        assert (ib, ob, len(bb)) == (ia, oa, len(ba)), f"i{cat}_dat: 레코드 {ib} 경계가 움직였다"
    changed = {i for i in range(len(before)) if before[i] != after[i]}
    expect = set()
    for (c, idx, name, fo, old) in wanted:
        if c == cat:
            expect |= {fo, fo + 1, fo + 2, fo + 3}
    stray = changed - expect
    assert not stray, f"i{cat}_dat: 의도하지 않은 바이트가 변했다 {sorted(stray)[:8]}"
    # 새 가격이 정말 1골드로 읽히는지 (게스트 산식 그대로)
    for (c, idx, name, fo, old) in wanted:
        if c != cat:
            continue
        v = struct.unpack_from("<I", after, fo)[0]
        assert v >> SELL_PRICE_SHIFT == 1, f"i{cat}_dat idx {idx}: 판매가가 1골드가 아니다"
    return len(expect) // 4


def rebuild(zbytes, member, transform):
    """zip 한 멤버만 바꾸고 나머지는 메타데이터까지 그대로 복사한다."""
    src = zipfile.ZipFile(io.BytesIO(zbytes))
    out = io.BytesIO()
    seen = False
    with zipfile.ZipFile(out, "w") as dst:
        for info in src.infolist():
            data = src.read(info.filename)
            if info.filename == member:
                data = transform(data)
                seen = True
            zi = zipfile.ZipInfo(info.filename, date_time=info.date_time)
            zi.compress_type = info.compress_type
            zi.external_attr = info.external_attr
            zi.internal_attr = info.internal_attr
            zi.create_system = info.create_system
            dst.writestr(zi, data)
    if not seen:
        raise SystemExit(f"{member} not found")
    return out.getvalue()


def main():
    zpath = sys.argv[1]
    apply_it = "--apply" in sys.argv
    zbytes = open(zpath, "rb").read()

    z = zipfile.ZipFile(io.BytesIO(zbytes))
    jars = [n for n in z.namelist() if n.lower().endswith(".jar")]
    if len(jars) != 1:
        raise SystemExit(f"expected exactly one jar, got {jars}")
    jar = jars[0]
    jb = z.read(jar)

    jz = zipfile.ZipFile(io.BytesIO(jb))
    dat_member = {}
    for n in jz.namelist():
        base = n.rsplit("/", 1)[-1]
        if base.startswith("i") and base.endswith("_dat"):
            try:
                dat_member[int(base[1:-4])] = n
            except ValueError:
                pass
    dats = {cat: jz.read(n) for cat, n in dat_member.items()}

    wanted = plan(dats)
    print(f"{zpath}: jar = {jar}, {len(dats)} item dats")
    print(f"캐시 마커 `{CASH_MARKER}` + 가격 0 인 행 {len(wanted)} 개 -> price {TARGET_PRICE} "
          f"(판매가 = {TARGET_PRICE} >> {SELL_PRICE_SHIFT} = 1골드)")
    bycat = {}
    for row in wanted:
        bycat.setdefault(row[0], []).append(row)
    for cat in sorted(bycat):
        names = ", ".join(r[2] for r in bycat[cat])
        print(f"  i{cat}_dat ({len(bycat[cat])}): {names}")

    total = 0

    def patch_jar(jarbytes):
        nonlocal total
        inner = zipfile.ZipFile(io.BytesIO(jarbytes))
        out = io.BytesIO()
        with zipfile.ZipFile(out, "w") as dst:
            for info in inner.infolist():
                data = inner.read(info.filename)
                cat = next((c for c, n in dat_member.items() if n == info.filename), None)
                if cat is not None and cat in bycat:
                    new = patch_blob(cat, data, wanted)
                    n = verify(cat, data, new, wanted)
                    total += n
                    print(f"  i{cat}_dat: {n} rows, {len(data)} -> {len(new)} bytes (동일)")
                    data = new
                zi = zipfile.ZipInfo(info.filename, date_time=info.date_time)
                zi.compress_type = info.compress_type
                zi.external_attr = info.external_attr
                zi.internal_attr = info.internal_attr
                zi.create_system = info.create_system
                dst.writestr(zi, data)
        return out.getvalue()

    newzip = rebuild(zbytes, jar, patch_jar)

    # 바깥 zip 의 다른 멤버는 바이트 동일해야 한다.
    z2 = zipfile.ZipFile(io.BytesIO(newzip))
    assert z.namelist() == z2.namelist(), "zip member list changed"
    for n in z.namelist():
        if n != jar and z.read(n) != z2.read(n):
            raise SystemExit(f"zip member {n} changed unexpectedly")
    # jar 의 다른 멤버도 바이트 동일해야 한다.
    ja, jbz = zipfile.ZipFile(io.BytesIO(z2.read(jar))), zipfile.ZipFile(io.BytesIO(jb))
    assert ja.namelist() == jbz.namelist(), "jar member list changed"
    touched = {dat_member[c] for c in bycat}
    for n in jbz.namelist():
        if n not in touched and jbz.read(n) != ja.read(n):
            raise SystemExit(f"jar member {n} changed unexpectedly")
    print(f"  검증 OK: {total} rows 변경, 그 외 zip/jar 멤버 전부 바이트 동일")

    outdir = _out_dir(zpath)
    os.makedirs(outdir, exist_ok=True)
    if apply_it:
        bak = os.path.join(outdir, os.path.basename(zpath) + ".preprice")
        if not os.path.exists(bak):
            shutil.copy2(zpath, bak)
            print(f"  backup -> {bak}")
        open(zpath, "wb").write(newzip)
        print(f"  APPLIED: {zpath}  {len(zbytes)} -> {len(newzip)} bytes")
    else:
        out = os.path.join(outdir, os.path.basename(zpath) + ".new")
        open(out, "wb").write(newzip)
        print(f"  dry run -> {out}  {len(zbytes)} -> {len(newzip)} bytes")


if __name__ == "__main__":
    main()
