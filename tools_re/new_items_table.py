"""Build the 신규 아이템 table page (HTML artifact) from new_items.json."""
import html
import json
import os
import sys

SCR = sys.argv[1] if len(sys.argv) > 1 else os.getcwd()  # the directory `list_new_items.py` wrote new_items.json into
d = json.load(open(os.path.join(SCR, "new_items.json"), encoding="utf-8"))
items = d["items"]
recipes = d["recipes"]
shard = d["shard"]

SLOT_ORDER = ["투구", "갑옷", "장갑", "신발", "방패", "스피어", "스워드", "나이프", "단총", "캐논", "암흑성", "광휘석", "혼/악세", "악세"]
def slot_key(s):
    return SLOT_ORDER.index(s) if s in SLOT_ORDER else 99

def esc(x):
    return html.escape(str(x))

def num(n):
    return f"{n:,}"

def opt_cell(t):
    if t == "—":
        return '<td class="opt none">—</td>'
    return f'<td class="opt">{esc(t)}</td>'

legacy = sorted([r for r in items if r["group"] == "legacy"], key=lambda r: (slot_key(r["slot"]), r["lv"], r["idx"]))
reaver = [r for r in items if r["group"] == "reaver"]
bossacc = [r for r in items if r["group"] == "bossacc"]

def equip_rows(rows, with_look=True):
    out = []
    for r in rows:
        unit = "성능" if r["cat"] == 12 else ("공격" if 4 <= r["cat"] <= 10 else "방어")
        look = ""
        if with_look:
            if r["sprite"] is None:
                look = '<td class="mono dim">—</td>'
            else:
                pal = "고유색" if r["palette"] == 255 else f"ph{r['palette']:03d}"
                look = f'<td class="mono">b4 {r["sprite"]} · {pal}</td>'
        sub = f' <span class="dim">/ {r["sub"]}</span>' if r.get("sub") else ""
        out.append(
            f'<tr><td class="slot">{esc(r["slot"])}</td>'
            f'<td class="name"><span class="q">{esc(r["colour"])}</span>{esc(r["name"])}</td>'
            f'<td class="mono">{r["cat"]:02x}/{r["idx"]:02x}</td>'
            f'<td class="n">{r["lv"]}</td><td class="n">{r["grade"]}</td>'
            f'<td class="n">{r["main"]}<span class="dim unit"> {unit}</span>{sub}</td>'
            f'{opt_cell(r["opt1"])}{opt_cell(r["opt2"])}'
            f'<td class="n">{num(r["price"])}</td>{look}</tr>'
        )
    return "\n".join(out)

def recipe_rows(rows):
    out = []
    for r in rows:
        base = esc(r["base"]) if r["base"] else '<span class="dim">없음 (진입 단)</span>'
        out.append(
            f'<tr><td class="mono">0f/{r["idx"]:02x}</td><td class="name">{esc(r["result"])}</td>'
            f'<td class="slot">{esc(r["slot"])}</td><td class="n">{r["lv"]}</td>'
            f'<td class="n">{r["shards"]}</td><td>{base}</td><td class="n">{num(r["fee"])}</td><td class="n">{r["rate"]}%</td></tr>'
        )
    return "\n".join(out)

by_slot = {}
for r in legacy:
    by_slot.setdefault(r["slot"], []).append(r)

sections = []
for slot in SLOT_ORDER:
    if slot not in by_slot:
        continue
    rows = by_slot[slot]
    sections.append(f'''
<section class="grp" id="s-{esc(slot)}">
  <h3>{esc(slot)} <span class="count">{len(rows)}행</span></h3>
  <div class="scroll"><table>
    <thead><tr><th>부위</th><th>이름</th><th>cat/idx</th><th class="n">Lv</th><th class="n">등급</th><th class="n">주스탯</th><th>옵션 1</th><th>옵션 2</th><th class="n">가격</th><th>외형 (스프라이트 · 팔레트)</th></tr></thead>
    <tbody>{equip_rows(rows)}</tbody>
  </table></div>
</section>''')

page = f'''<title>영웅서기3 신규 아이템 표</title>
<meta name="description" content="레거시 78행 · 조합서 78행 · 닉스코어의 파편 · 리버 8 · 보스 악세 2 — 출고 에셋에서 그대로 읽은 값">
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=IBM+Plex+Sans+KR:wght@400;600&family=IBM+Plex+Mono:wght@400;500&display=swap">
<style>
/* layout: one column of slot-grouped tables, a sticky slot index on top; data set in mono, names in Plex KR */
:root {{
  --bg: #f6f3ec; --fg: #221f1a; --muted: #6e6656; --line: #d9d2c3; --head: #ebe5d7;
  --accent: #8a1c2b; --accent-ink: #fff5f5; --pill: #e6dccb; --mono: "IBM Plex Mono", ui-monospace, Consolas, monospace;
  --sans: "IBM Plex Sans KR", "Malgun Gothic", "Apple SD Gothic Neo", system-ui, sans-serif;
}}
@media (prefers-color-scheme: dark) {{ :root:not([data-theme="light"]) {{
  --bg: #17161a; --fg: #ebe6dc; --muted: #a39c8e; --line: #34313a; --head: #24222a;
  --accent: #e0616f; --accent-ink: #1a0b0e; --pill: #2c2933; color-scheme: dark; }} }}
:root[data-theme="dark"] {{
  --bg: #17161a; --fg: #ebe6dc; --muted: #a39c8e; --line: #34313a; --head: #24222a;
  --accent: #e0616f; --accent-ink: #1a0b0e; --pill: #2c2933; color-scheme: dark; }}
body {{ background: var(--bg); color: var(--fg); font-family: var(--sans); margin: 0; padding-block: 24px 64px; padding-inline: 16px; line-height: 1.45; }}
main {{ max-width: 1180px; margin: 0 auto; }}
h1 {{ font-size: 1.6rem; font-weight: 600; margin: 0 0 4px; text-wrap: balance; }}
.lede {{ color: var(--muted); margin: 0 0 18px; max-width: 70ch; }}
.stats {{ display: flex; flex-wrap: wrap; gap: 8px 14px; margin: 0 0 22px; padding: 0; list-style: none; font-family: var(--mono); font-size: .86rem; }}
.stats li {{ background: var(--pill); border-radius: 999px; padding: 3px 12px; }}
.stats b {{ color: var(--accent); font-weight: 600; }}
nav.idx {{ position: sticky; top: env(safe-area-inset-top, 0px); z-index: 2; background: var(--bg); border-bottom: 1px solid var(--line); padding: 8px 0; margin-bottom: 8px; display: flex; flex-wrap: wrap; gap: 6px; }}
nav.idx a {{ font-size: .82rem; color: var(--fg); text-decoration: none; border: 1px solid var(--line); border-radius: 6px; padding: 2px 9px; }}
nav.idx a:hover, nav.idx a:focus-visible {{ border-color: var(--accent); outline: none; }}
h2 {{ font-size: 1.15rem; margin: 34px 0 6px; padding-top: 8px; border-top: 2px solid var(--accent); }}
h3 {{ font-size: 1rem; margin: 20px 0 6px; }}
.count {{ color: var(--muted); font-weight: 400; font-size: .85rem; margin-left: 6px; }}
.note {{ color: var(--muted); font-size: .9rem; margin: 0 0 10px; max-width: 78ch; }}
.scroll {{ overflow-x: auto; min-width: 0; }}
table {{ border-collapse: collapse; width: 100%; font-size: .88rem; font-variant-numeric: tabular-nums; }}
th, td {{ text-align: left; padding: 5px 8px; border-bottom: 1px solid var(--line); white-space: nowrap; vertical-align: top; }}
th {{ background: var(--head); font-weight: 600; font-size: .78rem; letter-spacing: .02em; text-transform: uppercase; color: var(--muted); position: sticky; top: 0; }}
th.n, td.n {{ text-align: right; font-family: var(--mono); }}
td.mono {{ font-family: var(--mono); font-size: .82rem; }}
td.name {{ font-weight: 600; }}
td.slot {{ color: var(--muted); }}
.q {{ color: var(--accent); font-family: var(--mono); margin-right: 2px; }}
.dim {{ color: var(--muted); font-weight: 400; }}
.unit {{ font-size: .75rem; }}
td.opt.none {{ color: var(--muted); text-align: center; }}
.legend {{ display: grid; grid-template-columns: repeat(auto-fit, minmax(240px, 1fr)); gap: 10px 22px; margin: 10px 0 0; padding: 12px 14px; background: var(--head); border-radius: 8px; font-size: .86rem; }}
.legend p {{ margin: 0; }}
.legend b {{ font-weight: 600; }}
@media (max-width: 480px) {{ body {{ padding-inline: 16px; }} table {{ font-size: .8rem; }} th, td {{ padding: 4px 6px; }} }}
</style>
<main>
<h1>영웅서기3 신규 아이템 표</h1>
<p class="lede">출고 에셋(<code>hero3_fixed.zip</code> vc410)의 <code>i0..i12_dat</code>·<code>i15_dat</code>에서 그대로 읽은 값입니다. 개정할 행은 이 표의 cat/idx 로 지목하면 됩니다.</p>
<ul class="stats">
  <li>레거시 <b>{len(legacy)}</b>행 (<code>"</code> 색)</li>
  <li>조합서 <b>{len(recipes)}</b>행 (cat 0f · 0x27~0x74)</li>
  <li>재료 <b>1</b>행 (닉스코어의 파편)</li>
  <li>리버 무기 <b>{len(reaver)}</b> · 보스 악세 <b>{len(bossacc)}</b> (75차)</li>
</ul>
<nav class="idx" aria-label="부위">
  {"".join(f'<a href="#s-{esc(s)}">{esc(s)}</a>' for s in SLOT_ORDER if s in by_slot)}
  <a href="#recipes">조합서</a><a href="#material">재료</a><a href="#old">리버·보스 악세</a>
</nav>

<h2>1. 레거시 장비 78행 (133차 · 139차 외형)</h2>
<p class="note">옵션 값은 게임의 i8 옵션 쌍(id, 값) 그대로입니다. 최대HP·물리공격·특수공격·쿨타임은 <b>%</b>(게임 게터가 %로 읽는다 — 134차 §6), 나머지는 고정치. 「외형」= 139차가 심은 스프라이트 번호(b4)와 팔레트(b5; 고유색 = 0xff). 악세는 그려지지 않습니다.</p>
<div class="legend">
  <p><b>등급</b> 스탯 블록 b7 (악세 b5). 5 = 최종티어와 같은 자리.</p>
  <p><b>주스탯</b> 방어구는 방어력, 무기는 공격력, 악세는 성능. <span class="dim">/ n</span> 은 보조 수치(b14,15).</p>
  <p><b>가격</b> 상점 판매가는 이 값의 1/4 (<code>price &gt;&gt; 2</code>).</p>
  <p><b>cat/idx</b> 아이템 id. <code>wie_grant.txt</code>·드랍표·조합서가 전부 이 쌍을 씁니다.</p>
</div>
{"".join(sections)}

<h2 id="recipes">2. 조합서 78행 (138차 · cat 0f)</h2>
<p class="note">원작 네트워크 조합서 50행(0x27~0x58) 자리를 덮어쓰고 28행을 더한 것. 재료 = 닉스코어의 파편 N + 베이스 1(진입 단은 베이스 없음). 수수료 8,000 · 성공률 100 %(결정 ⑮). 성공 시 조합서 1장 소모.</p>
<div class="scroll"><table>
  <thead><tr><th>cat/idx</th><th>결과물</th><th>부위</th><th class="n">Lv</th><th class="n">파편</th><th>베이스</th><th class="n">수수료</th><th class="n">성공률</th></tr></thead>
  <tbody>{recipe_rows(recipes)}</tbody>
</table></div>

<h2 id="material">3. 재료 1행 (85차)</h2>
<div class="scroll"><table>
  <thead><tr><th>cat/idx</th><th>이름</th><th>어디서</th><th class="n">가격</th></tr></thead>
  <tbody><tr><td class="mono">{shard["cat"]:02x}/{shard["idx"]:02x}</td><td class="name"><span class="q">{{</span>{esc(shard["name"])}</td><td>하늘성 퇴장 보상(파편 3~30) · 헬 드랍 표 밖 · 재료가방 직행</td><td class="n">{num(shard["price"])}</td></tr></tbody>
</table></div>

<h2 id="old">4. 75차 신규 — 리버 무기 8 · 보스 전용 악세 2</h2>
<p class="note">하드 전용 드랍(78차-C). 리버 = 킬당 5 % 8종 무작위, 악세 = 크레이지타이탄 · 다크니스이라 전용 5 %.</p>
<div class="scroll"><table>
  <thead><tr><th>부위</th><th>이름</th><th>cat/idx</th><th class="n">Lv</th><th class="n">등급</th><th class="n">주스탯</th><th>옵션 1</th><th>옵션 2</th><th class="n">가격</th><th>외형</th></tr></thead>
  <tbody>{equip_rows(reaver + bossacc)}</tbody>
</table></div>
</main>
'''
out = os.path.join(SCR, "new_items_table.html")
open(out, "w", encoding="utf-8").write(page)
print("wrote", out, len(page))
