// 메뉴 터치화의 순수 절반 (66차, 2026-08-23) — 게임 자신의 패널 기하와 배지 스캔.
//
// ── 이 파일이 하는 일 ────────────────────────────────────────────────────────
// 터치 좌표(논리 게스트px)를 게임이 이해하는 것으로 바꾼다:
//   * 화면의 몇 번째 행/칸인가 -> `Grid::hit` + 화면별 표(`MENU`·`INVEN`·`SKILL`·`QUEST`·
//     `SYSTEM`)와 런타임 유도(`choice_grid`·`confirm_grid`·`item_grid`)
//   * 게임이 스스로 그린 «1» «3» «7» «9» 배지 중 무엇을 눌렀는가 -> `badges` + `badge_at`
//   * 지금 화면이 행 목록인가 가방 격자인가 -> `cell_box` (게임이 그린 노란 칸 테두리)
// 커서를 쓰는 것도, 키를 합성하는 것도 여기가 아니다 (그건 게스트 메모리를 가진
// 워커의 일이다 — `emulator.rs`). 여기는 순수 산술이라 테스트가 전부 커버한다.
//
// ── 근거는 전부 실측이다 (wie_probe, LGT 6a352944, 66차) ────────────────────
// 패널: **176 x 160 고정, 상하좌우 중앙 정렬**. 496x240 에서 x 160..335 / y 40..199
//   이고, y=40·41·185·195·199 행 전체가 패널 자기 배경색 (33,16,0) 으로 균일했다
//   (즉 잘려 있지 않다). 63차-B 는 같은 값을 논리 4종(496x240 / 496x228 / 560x240 /
//   440x220)에서 얻었다 — `panel_origin` 의 두 나눗셈이 그 표 그대로다.
// 메뉴 행: 선택 잉크 (49,28,16) / 비선택 (206,202,197) 밴드가 dy 24·43·62·81·100·119
//   에서 각각 15행. 즉 `24 + 19i`, 높이 15, x 는 dx 24..151.
//   ★ 커서 바이트에 4를 쓰고 키를 한 번도 안 보냈는데 다음 프레임에 4행이
//   선택됐다 (행별 선택잉크 카운트 [0,0,0,0,119,0]), 그리고 합성 OK 하나로 그 화면이
//   열렸다 (screen 0 -> 5).
// 인벤 칸: 커서 박스 = **(13 + 19c, 22 + 19r, 17, 17)**, 8열 x 2행.
//   칸 0 -> 칸 3 포크의 프레임 diff 가 **정확히 100 px** (박스당 50)이고 bbox 가
//   x 13..29 / x 70..86, y 22..38 이었다 — 63차-B 의 정적 분석 수치와 일치.
// 배지: 잉크 **(214,178,148)** = 0xd6b294. 좌열 dx 15..25, 우열 dx 151..161 (둘 다
//   폭 11) 이 화면·해상도 무관. dy 는 다르다:
//     탭 «1»«3»    dy 2..12   (인벤·장비·스킬·퀘스트·상태 전부 동일)
//     페이지 «7»«9» 인벤 dy 131..141 · 스킬/퀘스트 dy 145..155 · 장비/상태 **없음**
//   그래서 페이지 배지는 찾아서 쓴다 (위치를 가정하지 않는다). 메뉴 리스트에는
//   배지가 하나도 없다 — 실측대로 스캔이 아무것도 못 찾으므로 타깃도 안 생긴다.
//
// ── 왜 이 파일이 의존성 0 인가 ──────────────────────────────────────────────
// `menutouch_selftest.rs` (repo 루트) 가 이 실제 소스를 `include!` 해서 검증한다 —
// 사본이 아니다 (`items_selftest.rs` 가 사본으로 어떻게 썩는지 44차가 보여줬다).
// `use crate::...` 를 추가하면 그 테스트가 죽는다.

/// 게임 패널의 고정 크기 (논리 게스트px). ⛔ 윈도우px 로 바꾸지 말 것 — 게임은
/// 프레임버퍼 크기에 맞춰 자기 크롬을 다시 앵커한다 (MIGRATION_LGT.md §4.5).
pub const PANEL_W: i32 = 176;
pub const PANEL_H: i32 = 160;

/// 게임의 한 화면에서 포커스가 움직이는 **격자**. 전부 66차-B 기기 실측(S9, 논리 492x239)이고,
/// 좌표는 패널 상대다. 리스트는 `cols = 1` 인 격자다 — 게임도 그렇게 다룬다(인덱스 하나).
///
/// `w`/`h` 는 게임이 그리는 **선택 테두리**의 크기다(노란 사각형 또는 반전된 행 박스). 히트
/// 영역은 그보다 넓다: 칸 사이 빈틈은 양쪽 칸이 반씩 나눠 갖고, 격자 바깥으로 `pad_*` 만큼 더
/// 받는다 — 엄지는 빈틈을 피할 수 없고, 그게 이 기능이 생긴 이유다.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Grid {
    /// 첫 칸 테두리의 좌상단.
    pub x0: i32,
    pub y0: i32,
    /// 테두리 크기.
    pub w: i32,
    pub h: i32,
    /// 칸 간격 (테두리 + 빈틈).
    pub pitch_x: i32,
    pub pitch_y: i32,
    pub cols: i32,
    pub rows: i32,
    /// 격자 바깥으로 더 받는 여유.
    pub pad_x: i32,
    pub pad_y: i32,
}

impl Grid {
    /// 이 격자의 칸 수. 커서 바이트가 절대 넘어서면 안 되는 값이다.
    pub const fn cells(&self) -> i32 {
        self.cols * self.rows
    }

    /// 한 축의 인덱스. 빈틈은 가까운 칸이 갖고, 격자+여유 밖은 `None`.
    fn axis(d: i32, origin: i32, size: i32, pitch: i32, n: i32, pad: i32) -> Option<i32> {
        let gap = (pitch - size).max(0);
        let lo = origin - gap / 2 - pad;
        let hi = origin - gap / 2 + pitch * n + pad; // 배타
        if d < lo || d >= hi {
            return None;
        }
        Some(((d - origin + gap / 2).max(0) / pitch).min(n - 1))
    }

    /// 패널 상대 좌표 -> 칸 인덱스 (**행 우선**: `row * cols + col`). 격자 밖이면 `None`.
    ///
    /// ⛔ 이 함수는 `0..cells()` 밖을 절대 돌려주지 않는다. 게임의 클램프는 키 처리 arm 안에만
    /// 있어서(63차-B §7-1) 범위 밖 커서를 쓰면 도달 불가능한 화면이 열린다.
    pub fn hit(&self, dx: i32, dy: i32) -> Option<u8> {
        let col = Self::axis(dx, self.x0, self.w, self.pitch_x, self.cols, self.pad_x)?;
        let row = Self::axis(dy, self.y0, self.h, self.pitch_y, self.rows, self.pad_y)?;
        Some((row * self.cols + col) as u8)
    }

    /// 인덱스가 이 격자 안인가 — 커서를 쓰기 전 마지막 관문.
    pub fn holds(&self, index: u8) -> bool {
        i32::from(index) < self.cells()
    }

    /// `(dx, dy)` 가 **어떤 칸의 좌상단과 정확히 일치**하면 그 인덱스. [`Grid::hit`] 와 달리
    /// 여유도 반올림도 없다 — "이 테두리가 이 격자의 것인가"를 묻는 데 쓴다.
    pub fn origin_index(&self, dx: i32, dy: i32) -> Option<u8> {
        let (ddx, ddy) = (dx - self.x0, dy - self.y0);
        if ddx < 0 || ddy < 0 || ddx % self.pitch_x != 0 || ddy % self.pitch_y != 0 {
            return None;
        }
        let (col, row) = (ddx / self.pitch_x, ddy / self.pitch_y);
        (col < self.cols && row < self.rows).then(|| (row * self.cols + col) as u8)
    }
}

/// **메뉴 6행** (`CLR` 로 열리는 그것). 선택은 반전된 어두운 박스 (49,28,16), 비선택은
/// (206,202,197) — 밴드가 dy 24·43·62·81·100·119 에서 각 15행, 잉크 x 24..151 (프로브 실측).
pub const MENU: Grid = Grid {
    x0: 24,
    y0: 24,
    w: 128,
    h: 15,
    pitch_x: 128,
    pitch_y: 19,
    cols: 1,
    rows: 6,
    pad_x: 14,
    pad_y: 6,
};

/// **인벤토리 8x2**. 커서 박스 (13 + 19c, 22 + 19r, 17x17) — 칸 0→3 포크의 프레임 diff 가
/// 정확히 100 px 였고 bbox 가 x 13..29 / 70..86, y 22..38 이었다(프로브 실측).
pub const INVEN: Grid = Grid {
    x0: 13,
    y0: 22,
    w: 17,
    h: 17,
    pitch_x: 19,
    pitch_y: 19,
    cols: 8,
    rows: 2,
    pad_x: 0,
    pad_y: 0,
};

/// **스킬 3x2**. 노란 테두리 42x21 이 x 34·81·128 / y 19·43 에 앉는다(기기 실측: 커서
/// 0·1·2·3·5 각각의 테두리 bbox). 인덱스 = `row*3 + col` — RIGHT 가 +1, DOWN 이 +3.
pub const SKILL: Grid = Grid {
    x0: 34,
    y0: 19,
    w: 42,
    h: 21,
    pitch_x: 47,
    pitch_y: 24,
    cols: 3,
    rows: 2,
    pad_x: 2,
    pad_y: 2,
};

/// **퀘스트 6행**. 노란 테두리 163x16 이 y 23 + 19i, x 6..168 (기기 실측: 커서 1 의 테두리가
/// y 42..57). 행 박스 배경 (74,53,41) 밴드도 같은 여섯 자리에서 나왔다.
pub const QUEST: Grid = Grid {
    x0: 6,
    y0: 23,
    w: 163,
    h: 16,
    pitch_x: 163,
    pitch_y: 19,
    cols: 1,
    rows: 6,
    pad_x: 4,
    pad_y: 4,
};

/// **SYSTEM 4행** (세이브 / 환경설정 / 도움말 / 메인메뉴). 선택은 메뉴와 같은 반전 박스이고
/// 밴드가 dy 32·52·72·92 에서 각 15행, x 34..141 (기기 실측). ★ 간격이 **20** 이다 — 메뉴의
/// 19 가 아니다. 화면별 표가 필요한 이유가 이것이다.
pub const SYSTEM: Grid = Grid {
    x0: 34,
    y0: 32,
    w: 108,
    h: 15,
    pitch_x: 108,
    pitch_y: 20,
    cols: 1,
    rows: 4,
    pad_x: 14,
    pad_y: 5,
};

/// 공용 리스트 위젯(상점·제련·…)의 행 기하. 66차-B 기기 실측, 세 샘플:
///
/// ```text
///   상점 구매/판매 (win 12, rows 2)  행 0 = dy 48   행 1 = dy 69
///   상점 카테고리  (win 12, rows 3)  행 0 = dy 37   행 1 = dy 58
///   제련 선택지    (win 13, rows 3)  행 0 = dy 37   행 1 = dy 58
/// ```
///
/// 전부 커서 0↔1 프레임 diff 로 잰 것이고(변경 픽셀 5,560개, x 19..157), 간격 21 · 높이 20 ·
/// x 는 세 샘플에서 **같다**. 다른 것은 시작점 하나뿐이고, 그건 **행 블록이 세로 중앙정렬**
/// 이기 때문이다 — 2행이면 48, 3행이면 37 로 정확히 반 칸(10.5px)씩 올라간다.
pub const CHOICE_X0: i32 = 19;
pub const CHOICE_W: i32 = 139;
pub const CHOICE_PITCH: i32 = 21;
pub const CHOICE_H: i32 = 20;
/// 중앙정렬의 상수: `행0 = (CHOICE_SPAN2 - 행수 * CHOICE_PITCH) / 2`. 두 값(2행·3행)이
/// 이 직선에 정확히 앉고, 세 번째 샘플(구매 카테고리 3행)이 같은 값을 냈다.
pub const CHOICE_SPAN2: i32 = 138;
/// 이보다 많은 행을 말하는 위젯은 우리가 본 적 없는 상태다 — 추측하지 않고 거절한다.
///
/// ★★ **이 값은 유도된 것이다** (66차-C 리뷰가 잡았다. 예전 값 `16` 은 바로 위 문장과
/// 모순이었다 — 실측한 것은 **2행과 3행뿐**이고 16 은 아무 근거가 없는 숫자였다):
/// 중앙정렬 직선 `y0 = (138 - 행수*21)/2` 가 패널 안에 앉는 최대 행수가 **6** 이다
/// (6행 → y0 6, 아래끝 132). 7행이면 `y0= -4` 로 **패널 위쪽 밖**에서 시작하고,
/// 9행이면 격자가 **160 dy 전부**를 자기 것이라고 말한다 — 즉 창틀·제목·빈 곳을 눌러도
/// 행으로 해석된다. 게임은 이 창을 열 때 이미 0행을 포커스하므로 그건 곧
/// **손가락이 닿지도 않은 행에 OK 를 합성**하는 것이고, 상점·제련에서 그 다음은 구매
/// 확인창이다. 아래 `choice_grid` 의 "패널 안에 앉는가" 검사가 같은 것을 한 번 더 막는다.
///
/// ⚠ 행수 바이트(`CHAR+0xa285`)의 writer 는 아직 모르고(`TOUCH_MENU_FEASIBILITY.md §8`),
/// 그 위젯은 **화면이 바뀌어도 앞 화면 값을 들고 있다**(`items::menu_state` 주석). 이 경로에는
/// `item_grid_action` 같은 픽셀 교차검증이 **없다** — 그래서 모르는 행수는 거절이 유일한 답이다.
/// 7행 이상을 실제로 그리는 화면을 만나면 **그 화면을 재서** 이 상수와 직선을 같이 고칠 것.
pub const CHOICE_ROWS_MAX: u8 = 6;

/// 행수를 아는 상태에서 그 위젯의 격자. 행수가 0 이거나 상한을 넘으면 `None`.
///
/// ⛔ 행수는 **게임에서 라이브로 읽어서** 넣어야 한다(`CHAR+0xa285`). 화면마다 하드코딩하면
/// 바로 어긋난다 — 같은 창(상점)이 2행짜리와 3행짜리를 둘 다 그린다.
pub fn choice_grid(rows: u8) -> Option<Grid> {
    if rows == 0 || rows > CHOICE_ROWS_MAX {
        return None;
    }
    let n = i32::from(rows);
    let y0 = (CHOICE_SPAN2 - n * CHOICE_PITCH) / 2;
    // ⛔⛔ 블록이 **패널 안에 앉아야** 한다. `y0 < 0` 은 "게임이 자기 패널 위쪽 밖에 0행을
    // 그렸다"는 뜻이라 모델이 그 상태를 설명하지 못한다는 증거이고, 그때 `Grid::axis` 는
    // 패널 전체를 받아들인다(= 아무데나 탭이 행이 된다). `CHOICE_ROWS_MAX` 와 겹치는
    // 검사이지만 겹치는 게 맞다 — 중앙정렬 상수를 누가 고치면 상한만으로는 다시 열린다.
    if y0 < 0 || y0 + n * CHOICE_PITCH > PANEL_H {
        return None;
    }
    Some(Grid {
        x0: CHOICE_X0,
        y0,
        w: CHOICE_W,
        h: CHOICE_H,
        pitch_x: CHOICE_W,
        pitch_y: CHOICE_PITCH,
        cols: 1,
        // ⛔ 여유를 넉넉하게 주지 말 것 (2026-08-23 제보: "상점이나 제련창에서 아무데나 눌러도
        // OK버튼과 같이 동작하는거같이 보여"). 게임은 이 창을 열 때 이미 0행을 포커스해 두므로,
        // 보이는 행보다 넓은 히트 영역은 곧 "빈 곳을 눌렀는데 확인됐다"가 된다. 히트 칸은
        // 간격(21) 전체를 덮으니 행 사이 빈틈은 이미 커버되고, 블록 바깥으로는 1px 만 받는다.
        rows: n,
        pad_x: 1,
        pad_y: 1,
    })
}

/// 게임이 격자 칸 커서로 그리는 **노란 테두리**의 잉크 (255,243,0). 실측(66차-B): 인벤·상점
/// 판매·제련 재료 선택 모두 같은 색으로 17x17 사각형을 그린다. 스킬은 42x21, 퀘스트는 163x16
/// 이라 크기로 갈린다.
pub const CELL_BOX_INK: u32 = 0x00ff_f300;
/// 칸 테두리로 인정할 크기 범위 (실측 17). 이 밖은 다른 화면의 선택 테두리다.
pub const CELL_BOX_MIN: i32 = 15;
pub const CELL_BOX_MAX: i32 = 19;
/// 가로줄로 볼 최소 길이.
///
/// ★★ **아래 가로줄의 길이는 개수 라벨의 자릿수가 정한다** (80차-E 실측). 66차-B 는 한 자리
/// 라벨만 재고 「10px 까지 짧아진다」로 적었는데, 라벨은 칸의 **오른쪽 아래**에서 왼쪽으로
/// 자라므로 두 자리가 되면 더 먹는다 — 같은 커서 칸을 두 화면에서 재면:
///
/// ```text
///   싱글가방  개수 `E`   아래줄 dx 71..80 = 10 px
///   네트워크가방 개수 `33` 아래줄 dx 71..75 =  5 px   <- 옛 하한 8 에 걸려 탈락
/// ```
///
/// 탈락하면 [`cell_box`] 가 `None` 을 내고 [`item_grid`] 도 같이 죽는데, 그 `None` 은
/// 79차-F 규칙에서 **「격자가 아니라 목록이다」**로 읽히므로 정반대의 뜻이 된다. 그래서
/// 하한은 **위 가로줄용**으로만 남기고, 수집은 [`CELL_EDGE_ANY`] 로 한다.
/// ⛔ 이 값을 올리지 말 것 — 개수가 두 자리인 아이템에 커서가 앉는 것은 흔한 일이다.
pub const CELL_EDGE_MIN: i32 = 8;
/// 가로줄 후보로 **수집**할 최소 길이. 아래 가로줄은 위 문단만큼 짧아질 수 있으므로 여기서
/// 자르면 안 된다 — 박스인지는 폭(`CELL_BOX_MIN..=CELL_BOX_MAX`)·세로줄·짝의 x 정렬이 정한다.
pub const CELL_EDGE_ANY: i32 = 3;

/// 패널 안에서 **칸 커서 테두리**를 찾아 그 좌상단을 돌려준다 (패널 상대). 없으면 `None`.
///
/// ★★ 이것이 "행 목록"과 "가방 격자"를 가리는 유일하게 믿을 수 있는 신호다 (66차-B). 메모리
/// 바이트로는 못 가린다 — 공용 위젯의 커서·행수는 화면이 바뀌어도 **낡은 값이 남고**,
/// `0xa249` 는 상점 판매창에서는 0 이 되지만 **제련 재료창에서는 12** 였다. 그래서 제련
/// 가방창의 탭이 "행 0 = 커서 0" 으로 읽혀 곧바로 OK 가 됐다(제보: "다른 아이템을 눌러도
/// 포커스된 아이템에서 OK 누른것처럼 동작").
///
/// 사각형만 인정한다: 위/아래 가로줄이 같은 x 범위이고, 그 사이 세로줄이 양쪽에 서 있어야
/// 한다. 아이템 이름이 노란 글자인 경우가 있어서(창고열쇠) 글자를 테두리로 오인하면 안 된다.
/// Why [`cell_box`] answered the way it did — every yellow run it collected, with the
/// verdict on each one that could be a TOP edge.
///
/// ★★ 2026-09-15. A player reported the box windows refusing to open on the warehouse
/// screens, and the tap log could only say `cell cursor false`. That is exactly the shape
/// of diagnosis 80차-B wrote the REFUSED line for: a gate that does not name the term
/// that refused costs a screenshot and a round trip. This names them.
///
/// ⛔ Diagnostic only — no decision is taken from it, and it is built only when the
/// caller is already about to log.
pub fn cell_box_report(px: &[u32], lw: i32, lh: i32) -> String {
    if lw <= 0 || lh <= 0 || px.len() != (lw as usize) * (lh as usize) {
        return "no frame".to_string();
    }
    let Some((ox, oy)) = panel_origin(lw, lh) else {
        return "no panel".to_string();
    };
    let at = |dx: i32, dy: i32| -> bool {
        let (x, y) = (ox + dx, oy + dy);
        x >= 0 && x < lw && y >= 0 && y < lh && px[(y as usize) * (lw as usize) + x as usize] & 0x00ff_ffff == CELL_BOX_INK
    };
    let mut runs: Vec<(i32, i32, i32)> = Vec::new();
    for dy in 0..PANEL_H {
        let mut start: Option<i32> = None;
        for dx in 0..=PANEL_W {
            let on = dx < PANEL_W && at(dx, dy);
            match (start, on) {
                (None, true) => start = Some(dx),
                (Some(a), false) => {
                    if dx - a >= CELL_EDGE_ANY {
                        runs.push((dy, a, dx - 1));
                    }
                    start = None;
                }
                _ => {}
            }
        }
    }
    let mut out = format!("ink {CELL_BOX_INK:#08x} runs {}", runs.len());
    // The pairing the detector actually makes, and how the VERTICALS voted — the term
    // that decides a box, and the one the run list cannot show (a vertical is one pixel
    // per row, far under `CELL_EDGE_ANY`).
    for &(top, a, b) in runs.iter().take(4) {
        for &(bot, a2, b2) in runs.iter().take(4) {
            let h = bot - top + 1;
            if bot <= top || !(CELL_BOX_MIN..=CELL_BOX_MAX).contains(&h) || a.max(a2) > b.min(b2) {
                continue;
            }
            let (left, right) = (a.min(a2) - 1, b.max(b2) + 1);
            let l = (top + 1..bot).filter(|&dy| at(left, dy)).count();
            let r = (top + 1..bot).filter(|&dy| at(right, dy)).count();
            let both = (top + 1..bot).filter(|&dy| at(left, dy) && at(right, dy)).count();
            let _ = both;
            out.push_str(&format!(
                " || pair dy{top}/{bot} h{h} x{left}..{right} w{} L{l} R{r} both{both}/{}",
                right - left + 1,
                h - 1
            ));
        }
    }
    for &(dy, a, b) in runs.iter().take(6) {
        let w = b - a + 3;
        let len = b - a + 1;
        let why = if len < CELL_EDGE_MIN {
            "short"
        } else if !(CELL_BOX_MIN..=CELL_BOX_MAX).contains(&w) {
            "width"
        } else {
            "top-ok"
        };
        out.push_str(&format!(" | dy{dy} x{a}..{b} len{len} w{w} {why}"));
    }
    out
}

pub fn cell_box(px: &[u32], lw: i32, lh: i32) -> Option<(i32, i32)> {
    if lw <= 0 || lh <= 0 || px.len() != (lw as usize) * (lh as usize) {
        return None;
    }
    let (ox, oy) = panel_origin(lw, lh)?;
    let at = |dx: i32, dy: i32| -> bool {
        let (x, y) = (ox + dx, oy + dy);
        x >= 0 && x < lw && y >= 0 && y < lh && px[(y as usize) * (lw as usize) + x as usize] & 0x00ff_ffff == CELL_BOX_INK
    };
    // 가로줄 후보들. ★ 실측 모양(66차-B, 상점 판매창의 칸 0):
    //     dy 18       x 14..28   (위 가로줄 — 양끝이 1px 안쪽으로 들어간다)
    //     dy 19..33   x 13 과 29 (세로줄 — 가로줄보다 1px 밖)
    //     dy 34       x 14..23   (아래 가로줄 — 아이템 개수 라벨에 **가려서 짧다**)
    // 그래서 위/아래를 짝지을 때 **왼쪽 끝만** 맞추고 오른쪽 끝은 위 가로줄에서 가져온다.
    let mut runs: Vec<(i32, i32, i32)> = Vec::new(); // (dy, x0, x1)
    for dy in 0..PANEL_H {
        let mut start: Option<i32> = None;
        for dx in 0..=PANEL_W {
            let on = dx < PANEL_W && at(dx, dy);
            match (start, on) {
                (None, true) => start = Some(dx),
                (Some(a), false) => {
                    if dx - a >= CELL_EDGE_ANY {
                        runs.push((dy, a, dx - 1));
                    }
                    start = None;
                }
                _ => {}
            }
        }
    }
    // ★★★ 2026-09-15 — THE BOX IS ITS VERTICALS, not the length of either horizontal.
    //
    // 80차-E had already found that the BOTTOM bar is shortened by the stack-count label.
    // The 가방칸 셋 showed that the TOP one is not safe either: their icon is drawn over the
    // cursor's top-left corner, so on the warehouse grid the top bar came back as
    // `dy23 x20..28` — nine pixels of a fifteen-pixel edge (device, S9 vc320, the report
    // `cell_box_report` prints). The old pairing asked the TOP bar for the box's width and
    // its LEFT end, so it refused the box outright and the whole item grid went with it —
    // player: *"여전히 상세보기, 버리기, 올리기밖에없어"*.
    //
    // So the two bars only say WHERE to look: the box spans from the leftmost of the two
    // to the rightmost, and what confirms it is the pair of VERTICALS — which nothing
    // draws over, because they sit one pixel outside the cell.
    //
    // ⛔ Do not go back to measuring the width off one bar. Either bar can be cut, and
    // which one depends on the item's art and on how many digits its count has.
    for &(top, a, b) in &runs {
        for &(bot, a2, b2) in &runs {
            let h = bot - top + 1;
            if bot <= top || !(CELL_BOX_MIN..=CELL_BOX_MAX).contains(&h) {
                continue;
            }
            // The two bars must overlap — otherwise they are edges of different cells.
            if a.max(a2) > b.min(b2) {
                continue;
            }
            let (left, right) = (a.min(a2) - 1, b.max(b2) + 1);
            let w = right - left + 1;
            if !(CELL_BOX_MIN..=CELL_BOX_MAX).contains(&w) {
                continue;
            }
            // 세로줄: 두 가로줄 사이 행의 절반 이상에서 **가로줄보다 1px 밖**이 노랗다.
            // ⛔ 이것이 유일한 확인이다 — 가로줄 길이는 그리기에 따라 잘린다.
            //
            // ★★★ 2026-09-15 — 두 세로줄을 **따로** 센다. 예전 규칙은 「같은 행에서 양쪽이
            // 동시에 노랗다」였는데, 기기 실측(S9 vc321, 창고 격자의 가방칸 셀)이
            // `L9 R10 both4/16` 이었다: 각 세로줄은 열여섯 행 중 아홉·열 행에서 멀쩡한데
            // **가려지는 행이 서로 달라서** 동시 만족이 넷뿐이다. 왼쪽은 아이템 그림이,
            // 오른쪽은 개수 라벨이 갉아먹기 때문에 겹칠 이유가 없다.
            // ⛔ 동시 조건으로 되돌리지 말 것 — 그러면 그림이 있는 칸의 커서를 영영 못 찾는다.
            let half = h - 2;
            let l = (top + 1..bot).filter(|&dy| at(left, dy)).count() as i32;
            let r = (top + 1..bot).filter(|&dy| at(right, dy)).count() as i32;
            if l * 2 >= half && r * 2 >= half {
                return Some((left, top));
            }
        }
    }
    None
}

/// 확인창(`예 / 아니오`)의 칸 기하 — 같은 공용 위젯인데 **가로**로 눕는다.
/// 66차-B 기기 실측(상점 구매 확인창, win 12, `axis=0`, `rows=2`): 커서 0↔1 프레임 diff 가
/// x **32..83** 과 **92..143**(각 폭 52, 간격 60), y **71..84**(높이 14) 였고, RIGHT 로
/// 0→1 이 움직였다 — DOWN 은 아무 일도 하지 않는다.
///
/// ⛔ 칸 수 **2 만** 지원한다. 게임이 그 창을 3칸으로 그리는 걸 본 적이 없고, 가로 중앙정렬의
/// 상수를 한 샘플로는 못 정한다 — 모르는 것은 거절이 이 기능의 규칙이다.
pub const CONFIRM_X: [i32; 2] = [32, 92];
pub const CONFIRM_W: i32 = 52;
pub const CONFIRM_Y: i32 = 71;
pub const CONFIRM_H: i32 = 14;
/// 질문이 **두 줄**일 때의 칸 윗줄 (117차-D). 귀환서의 「최근 귀환장소로 이동하겠습니까?」는 줄이
/// 하나 더 있어 예/아니오가 그만큼 내려온다 — 프로브 프레임 실측: 선택된 「예」 채움이
/// dx 32..83 / **dy 81..94**(52x14, [`CONFIRM_Y`] 판과 폭·높이·x 는 같다). 감지기 셋은 두 줄을
/// 다 시험한다. ⛔ 한 줄 판을 이 값으로 바꾸지 말 것 — 저장·구매 확인창은 71 이 맞다.
pub const CONFIRM_Y_2LINE: i32 = 81;

/// 확인창의 격자. 칸 수가 2 가 아니면 `None`. 질문이 한 줄인 창의 것([`CONFIRM_Y`]).
pub fn confirm_grid(cols: u8) -> Option<Grid> {
    confirm_grid_at(cols, CONFIRM_Y)
}

/// [`confirm_grid`] 의 윗줄을 고른 판 — [`CONFIRM_Y`] 또는 [`CONFIRM_Y_2LINE`].
pub fn confirm_grid_at(cols: u8, y0: i32) -> Option<Grid> {
    if cols != 2 {
        return None;
    }
    Some(Grid {
        x0: CONFIRM_X[0],
        y0,
        w: CONFIRM_W,
        h: CONFIRM_H,
        pitch_x: CONFIRM_X[1] - CONFIRM_X[0],
        pitch_y: CONFIRM_H,
        cols: 2,
        rows: 1,
        // ⛔ 세로 여유를 크게 주면 **질문 문구를 누른 것이 `예`가 된다** — 구매·삭제 확인창에서
        // 그건 사고다. 잰 값(y 71..84)에 1px 만 얹는다.
        pad_x: 2,
        pad_y: 1,
    })
}

/// 확인창의 **선택된** 칸이 칠해지는 잉크 — [`CONFIRM3_SEL_INK`] 와 같은 색이다.
/// (실측 80차-G: 인게임 세이브 확인창의 「예」 배경이 dx 32..83 / dy 71..84 전체를 이 색으로
/// 채운다. 66차-B 가 [`SPIN_INK`] 주석에 *"확인창의 선택된 `예` 배경이 같은 색이고 52px 짜리
/// 런을 만든다"* 고 적어 둔 것이 같은 관측이다.)
///
/// 칸을 인정할 최소 채움 비율 (실측 0.94; 「예」 글자가 흰색으로 얹혀 있어서 1.0 은 안 된다).
pub const CONFIRM_FILL_NUM: i32 = 7;
pub const CONFIRM_FILL_DEN: i32 = 10;
/// 실측 위치와의 허용 오차 (px). 0 으로 두면 안 되는 이유는 없지만, 다른 논리 해상도에서
/// 반올림이 1px 을 움직일 수 있다.
pub const CONFIRM_CELL_TOL: i32 = 1;
/// 한 칸의 잉크를 모을 창을 그 칸 밖으로 이만큼 넓힌다.
///
/// ⛔ 두 칸 **사이의 빈 칸(실측 8px: 칸0 이 dx 83 에서 끝나고 칸1 이 92 에서 시작)보다 작아야
/// 한다** — 아니면 한 칸의 창이 옆 칸의 채움을 삼켜 바운딩 박스가 커지고, 그 칸이 「안 맞는다」
/// 로 조용히 떨어진다. 그러면 **둘 다 칠해진 프레임이 한 칸만 찾은 것으로 보인다**(셀프테스트가
/// 실제로 그걸 잡았다). 동시에 0 보다는 커야 칸 밖으로 삐져나온 잉크를 감지한다.
pub const CONFIRM_CELL_MARGIN: i32 = 3;

/// 게임이 **채워 그린** 확인창 칸의 인덱스(0 = 예, 1 = 아니오). 없거나 둘 다면 `None`.
///
/// ★★★ 80차-G. 제보(2026-09-05): *"인게임 -> 시스템 -> 세이브 … 저장하시겠습니까? 라고 떴을때
/// 예 혹은 아니오 선택해도 팝업창에서 동작을 안하고 갑자기 뒤에있는 화면에 터치가돼"*.
///
/// 그 창은 **모달이 아니라 공용 리스트 위젯의 가로 모드**다. 기기 실측(vc255, S9, 확인창이 떠
/// 있는 상태의 `focus probe`):
/// ```text
///   overlay=1 screen=Some(9) win=3 | list cur=Some(0) rows=Some(2) axis=Some(0) | choice=1
/// ```
/// `axis=0`(가로) · `rows=2` · `cur=0`, 그리고 게임 자신의 선택 플래그(`CHAR+0xa224`)가 1 이다 —
/// 79차-E 가 대화 선택지에서 확정한 바로 그 바이트이고, 게임이 선택을 적용할 때 스스로 0 으로
/// 지운다. 그리고 픽셀은 [`confirm_grid`] 가 66차-B 에 상점 구매 확인창에서 재 둔 격자와
/// **정확히 일치**한다(칸0 dx 32..83 · 칸1 dx 92..143 · dy 71..84).
///
/// ⛔ 그래서 이 창은 [`confirm_pair`]·[`confirm_pair_shell`] 이 아는 **모달 페인터가 아니다**:
/// 모달 쪽은 40x16 이고 안 선택된 쪽에 `#9c7d73` 외곽선을 그리는데, 이 창은 **52x14** 이고
/// 안 선택된 쪽은 **상자 없이 흐린 글자**뿐이다(실측 `#a5827b`). 그래서 감지기가 하나 더 있다.
#[allow(dead_code)] // the one-line form: kept for `menutouch_selftest` and `menutouch_framecheck`
pub fn confirm_cell(px: &[u32], lw: i32, lh: i32) -> Option<u8> {
    confirm_cell_at(px, lw, lh, CONFIRM_Y)
}

/// [`confirm_cell`] with the pair's top row given — [`CONFIRM_Y`] for a one-line question,
/// [`CONFIRM_Y_2LINE`] for a two-line one (117차-D).
pub fn confirm_cell_at(px: &[u32], lw: i32, lh: i32, y0: i32) -> Option<u8> {
    if lw <= 0 || lh <= 0 || px.len() != (lw as usize) * (lh as usize) {
        return None;
    }
    let (ox, oy) = panel_origin(lw, lh)?;
    let g = confirm_grid_at(2, y0)?;
    let mut found = None;
    for cell in 0..2i32 {
        let x0 = g.x0 + cell * g.pitch_x;
        // 이 칸의 것만 센다 — 다른 칸이나 화면의 같은 색은 이 칸의 증거가 아니다.
        let m = CONFIRM_CELL_MARGIN;
        let mut n = 0;
        let (mut bx0, mut by0, mut bx1, mut by1) = (PANEL_W, PANEL_H, -1, -1);
        for dy in (g.y0 - m).max(0)..(g.y0 + g.h + m).min(PANEL_H) {
            for dx in (x0 - m).max(0)..(x0 + g.w + m).min(PANEL_W) {
                let (x, y) = (ox + dx, oy + dy);
                if x < 0 || x >= lw || y < 0 || y >= lh {
                    continue;
                }
                if px[(y as usize) * (lw as usize) + x as usize] & 0x00ff_ffff != CONFIRM3_SEL_INK {
                    continue;
                }
                n += 1;
                bx0 = bx0.min(dx);
                by0 = by0.min(dy);
                bx1 = bx1.max(dx);
                by1 = by1.max(dy);
            }
        }
        if bx1 < bx0 {
            continue;
        }
        // 바운딩 박스가 **그 칸 그 자체**여야 하고, 안이 실제로 채워져 있어야 한다.
        let fits = (bx0 - x0).abs() <= CONFIRM_CELL_TOL
            && (bx1 - (x0 + g.w - 1)).abs() <= CONFIRM_CELL_TOL
            && (by0 - g.y0).abs() <= CONFIRM_CELL_TOL
            && (by1 - (g.y0 + g.h - 1)).abs() <= CONFIRM_CELL_TOL;
        let area = (bx1 - bx0 + 1) * (by1 - by0 + 1);
        if !fits || n * CONFIRM_FILL_DEN < area * CONFIRM_FILL_NUM {
            continue;
        }
        // 둘 다 칠해져 있으면 어느 쪽이 선택인지 말할 수 없다 — 아무것도 안 한다.
        if found.is_some() {
            return None;
        }
        found = Some(cell as u8);
    }
    found
}

/// 공용 위젯 확인창의 두 버튼 — `(선택된 것, 나머지)`. 없으면 `None`.
///
/// [`confirm_cell`] 이 답한 칸이 선택이고, 나머지는 [`confirm_grid`] 의 다른 칸이다.
/// 기하는 게임에서 잰 상수이므로 접거나 유도할 것이 없다.
pub fn confirm_pair_widget(px: &[u32], lw: i32, lh: i32) -> Option<(Box2, Box2)> {
    // ★ 117차-D: a one-line question first (저장 · 구매 · 삭제), then the two-line one
    //   (귀환서). The two rows do not overlap (71..84 vs 81..94 share only 4 rows, and a
    //   cell has to FIT its row within 1 px), so the first that answers is the one drawn.
    [CONFIRM_Y, CONFIRM_Y_2LINE].into_iter().find_map(|y0| {
        let cell = confirm_cell_at(px, lw, lh, y0)?;
        let g = confirm_grid_at(2, y0)?;
        let rect = |c: i32| Box2 { dx: g.x0 + c * g.pitch_x, dy: g.y0, w: g.w, h: g.h };
        let sel = i32::from(cell);
        Some((rect(sel), rect(1 - sel)))
    })
}

// ── 게임의 메시지 박스 틀 (117차-D) ─────────────────────────────────────────────
//
// 귀환서의 「최근 귀환장소로 이동하겠습니까?」는 공용 선택 위젯이 **패널 안에** 그리는 메시지
// 박스다 — 프로브 프레임 실측: 2px 짜리 틀 `#210c10` 이 **dx 10..166 / dy 25..125** 를 감싸고
// (157x101, 위·아래 줄과 좌·우 열이 통째로 그 색), 그 안쪽 하이라이트 `#f7ebe6` 16..160/31..108.
// 인벤토리 페이지·동작 팝업 프레임에는 그 색이 **한 픽셀도 없다**(1990·2300 프레임 0개).
// 소비가방 창은 그 패널을 덮는 호스트 오버레이라, 게임 팝업을 「그대로」 보이려면 창에 이
// 사각형만큼 구멍을 내고 게임 프레임을 다시 그린다(`App::reveal_game_dialog`).
/// 게임 메시지 박스의 틀 색.
pub const GAME_DIALOG_FRAME_INK: u32 = 0x0021_0c10;
/// 틀로 인정할 최소 크기 — 실측 157x101 의 절반 남짓. 글자 한두 픽셀은 통과 못 한다.
pub const GAME_DIALOG_MIN_W: i32 = 100;
pub const GAME_DIALOG_MIN_H: i32 = 40;

/// 패널 안에 게임의 메시지 박스가 떠 있으면 그 **틀의 바운딩 박스**(패널 상대). 없으면 `None`.
///
/// 판정 = [`GAME_DIALOG_FRAME_INK`] 픽셀의 바운딩 박스가 최소 크기 이상이고, 그 **윗줄과
/// 아랫줄이 통째로 그 색**이어야 한다(틀은 닫힌 사각형이다 — 흩어진 같은 색 점들은 여기서
/// 떨어진다). FAIL CLOSED: 모르면 아무것도 드러내지 않는다.
pub fn game_dialog_frame(px: &[u32], lw: i32, lh: i32) -> Option<Box2> {
    if lw <= 0 || lh <= 0 || px.len() != (lw as usize) * (lh as usize) {
        return None;
    }
    let (ox, oy) = panel_origin(lw, lh)?;
    let ink = |dx: i32, dy: i32| -> bool {
        let (x, y) = (ox + dx, oy + dy);
        x >= 0 && x < lw && y >= 0 && y < lh && px[(y as usize) * (lw as usize) + x as usize] & 0x00ff_ffff == GAME_DIALOG_FRAME_INK
    };
    let (mut x0, mut y0, mut x1, mut y1) = (PANEL_W, PANEL_H, -1, -1);
    for dy in 0..PANEL_H {
        for dx in 0..PANEL_W {
            if ink(dx, dy) {
                x0 = x0.min(dx);
                y0 = y0.min(dy);
                x1 = x1.max(dx);
                y1 = y1.max(dy);
            }
        }
    }
    if x1 < x0 || x1 - x0 + 1 < GAME_DIALOG_MIN_W || y1 - y0 + 1 < GAME_DIALOG_MIN_H {
        return None;
    }
    let solid = |dy: i32| (x0..=x1).all(|dx| ink(dx, dy));
    if !solid(y0) || !solid(y1) {
        return None;
    }
    Some(Box2 { dx: x0, dy: y0, w: x1 - x0 + 1, h: y1 - y0 + 1 })
}

// ── 타이틀 카루셀 `≪ 항목 ≫` (80차-K) ──────────────────────────────────────────
//
// ★★★ 플레이어 제보(2026-09-05): *"메인화면에서도 터치가 안되고있어. 다음 화살표 누르면
// 다음인 네트워크창고, 환경설정 이런식으로 메인화면 내에서 이동이 되어야하는데 안되고있고 …
// 게임종료 눌러서 게임 종료하시겠습니까? 에서도 터치 안되고있어"*.
//
// **그 둘은 같은 위젯이다.** 「게임을 종료하시겠습니까?」의 예/아니오도 `≪ 예 ≫` 카루셀로
// 그려진다(헤드리스 프로브 실측) — 그래서 확인창 감지기 셋이 아무것도 못 찾았던 것이다.
//
// ── 실측 (wie_probe, 논리 496x240, 패널 상대) ───────────────────────────────────
// ```text
//   게임시작 · 네트워크창고 · 환경설정 · 게임방법 · 게임종료 · 「종료하시겠습니까?」의 예
//     ≪  dx 41..52   ≫  dx 123..134     dy 135..139     잉크 #fffbff  각 18 px
// ```
// 여섯 화면에서 **바이트 동일**이고, OPTION·HELP·모달 확인창에는 이 색이 **한 픽셀도 없다**.
// 그래서 이 잉크 하나가 「지금 카루셀이 떠 있다」의 신호다.
/// 카루셀 화살표의 잉크 — 크림 흰색. ⛔ `0xffffff` 와 다르다(그쪽은 본문 글자다).
pub const CAROUSEL_INK: u32 = 0x00ff_fbff;
/// 화살표 하나로 볼 열 런의 폭 (실측 12). ⛔ 하한이 필요하다 — 타이틀 아트에 같은 색 2px
/// 짜리 조각이 하나 있다(게임방법 프레임 dx 38..39).
pub const CAROUSEL_ARROW_W_MIN: i32 = 10;
pub const CAROUSEL_ARROW_W_MAX: i32 = 14;
/// 히트 밴드를 잉크 위아래로 이만큼 키운다 — 글리프가 5px 밖에 안 된다.
pub const CAROUSEL_PAD_Y: i32 = 9;
/// 가운데 라벨 상자와 화살표 상자 사이의 여유.
pub const CAROUSEL_GAP: i32 = 3;

/// 타이틀 카루셀의 세 타깃 — 이전(`≪`) · 항목(가운데) · 다음(`≫`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Carousel {
    pub prev: Box2,
    pub pick: Box2,
    pub next: Box2,
}

/// 지금 화면에 카루셀이 있으면 그 세 타깃. 없으면 `None`.
///
/// ★ 화살표는 5px 짜리 글리프지만 히트 영역은 **그 줄 전체를 셋으로 나눈 것**이다: 왼쪽 끝부터
/// `≪` 까지가 이전, `≫` 부터 오른쪽 끝까지가 다음, 그 사이가 항목이다. 그 줄에는 다른 것이
/// 아무것도 없다(실측: 여섯 화면 전부).
pub fn title_carousel(px: &[u32], lw: i32, lh: i32) -> Option<Carousel> {
    if lw <= 0 || lh <= 0 || px.len() != (lw as usize) * (lh as usize) {
        return None;
    }
    let (ox, oy) = panel_origin(lw, lh)?;
    let ink = |dx: i32, dy: i32| -> bool {
        let (x, y) = (ox + dx, oy + dy);
        x >= 0 && x < lw && y >= 0 && y < lh && px[(y as usize) * (lw as usize) + x as usize] & 0x00ff_ffff == CAROUSEL_INK
    };
    // 잉크가 있는 열들을 런으로 묶고, 화살표 폭인 것만 남긴다.
    let mut runs: Vec<(i32, i32)> = Vec::new();
    let mut start: Option<i32> = None;
    for dx in 0..=PANEL_W {
        match (start, dx < PANEL_W && (0..PANEL_H).any(|dy| ink(dx, dy))) {
            (None, true) => start = Some(dx),
            (Some(a), false) => {
                let w = dx - a;
                if (CAROUSEL_ARROW_W_MIN..=CAROUSEL_ARROW_W_MAX).contains(&w) {
                    runs.push((a, dx - 1));
                }
                start = None;
            }
            _ => {}
        }
    }
    // ⛔ 정확히 둘 — 하나뿐이거나 셋이면 우리가 아는 화면이 아니다.
    let [(l0, l1), (r0, r1)] = runs[..] else { return None };
    if r0 <= l1 {
        return None;
    }
    let rows: Vec<i32> = (0..PANEL_H)
        .filter(|&dy| (l0..=l1).chain(r0..=r1).any(|dx| ink(dx, dy)))
        .collect();
    let (&y0, &y1) = (rows.first()?, rows.last()?);
    let top = (y0 - CAROUSEL_PAD_Y).max(0);
    let bot = (y1 + CAROUSEL_PAD_Y).min(PANEL_H - 1);
    let band = |dx: i32, w: i32| Box2 { dx, dy: top, w, h: bot - top + 1 };
    let pick0 = l1 + 1 + CAROUSEL_GAP;
    let pick1 = r0 - 1 - CAROUSEL_GAP;
    (pick1 > pick0).then(|| Carousel {
        prev: band(0, pick0),
        pick: band(pick0, pick1 - pick0 + 1),
        next: band(pick1 + 1, PANEL_W - pick1 - 1),
    })
}

// ── 페이저 헤더 `◀ 제목 ▶` — HELP · FORCE · CLASS (80차-L, 80차-N 에서 일반화) ────
//
// 플레이어 요청(2026-09-05): *"게임 방법창 진행하자"* → *"빈슬롯에서 게임 새로 시작할때도
// 전부 터치 제대로 동작해야해"*.
//
// ★★★ **세 화면이 같은 위젯이다** (80차-N, 프로브 프레임 실측): 게임방법(HELP), 그리고 새
// 캐릭터 생성의 **FORCE**(국가)와 **CLASS**(직업). 셋 다 `◀ 제목상자 ▶` 헤더에 본문·스크롤바가
// 붙는다. 삼각형 좌표까지 같다(HELP dx 18..24, FORCE·CLASS dx 19..25 / 둘 다 150..156).
//
// ── 실측 (wie_probe, 논리 496x240, 패널 상대) ───────────────────────────────────
// ```text
//   ◀  dx  18..24   ▶  dx 150..156      dy 21..31   잉크 #311008 (7x11 삼각형)
//   제목 상자  dx 32..143  dy 19..32                잉크 #311408 (흰 글자가 얹힌다)
//   헤더 밑 흰 줄 dy 33 · 본문 dy 34..147 · 창 밑단 흰 줄 dy 148
//   스크롤바 dx 166..172
// ```
// 키도 프로브로 확인했다: **LEFT/RIGHT = 페이지**(「기본 인터페이스」 → 「네오 솔티아 소개」),
// **UP/DOWN = 본문 스크롤**(같은 페이지의 뒷부분이 나오고 스크롤바 썸이 내려간다).
/// 페이지 화살표의 잉크.
pub const PAGER_TRI_INK: u32 = 0x0031_1008;
/// 그 사이 제목 상자의 잉크. ⛔ 화살표와 **4밖에 안 다르다**(초록 채널) — 둘 다 정확한 값으로
/// 비교할 것.
pub const PAGER_TITLE_INK: u32 = 0x0031_1408;
/// 삼각형 하나로 볼 열 런의 폭 (실측 7).
pub const PAGER_TRI_W_MIN: i32 = 5;
pub const PAGER_TRI_W_MAX: i32 = 9;
/// 두 화살표 사이가 제목 상자여야 한다 — 그 비율.
///
/// ⛔ 3/4 이면 **FORCE 를 놓친다**: 그 제목 상자에는 국가 깃발 아이콘이 얹혀 있어서 가운데
/// 줄의 채움이 84/124 = **0.68** 이다(CLASS 0.85 · HELP 0.81). 3/5 는 그 셋 아래이고, 아무것도
/// 없는 화면(0)과는 여전히 한참 위다.
pub const PAGER_TITLE_FILL_NUM: i32 = 3;
pub const PAGER_TITLE_FILL_DEN: i32 = 5;
/// 본문(스크롤) 영역 — 헤더 밑 흰 줄 다음부터 창 밑단 흰 줄 앞까지 (실측).
pub const PAGER_BODY_TOP: i32 = 34;
pub const PAGER_BODY_BOT: i32 = 147;
/// 헤더 밴드를 삼각형 위아래로 이만큼 키운다.
pub const PAGER_HEADER_PAD: i32 = 6;

/// 페이저 창의 다섯 타깃 — 이전/다음, 제목(확정), 본문 위/아래 스크롤.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PagerHeader {
    pub prev: Box2,
    /// 제목 상자 — **OK**. FORCE·CLASS 에서는 「이걸로 한다」이고, HELP 에서는 무동작이다
    /// (프로브 실측: HELP 에서 OK 를 눌러도 화면이 한 픽셀도 안 바뀐다).
    pub pick: Box2,
    pub next: Box2,
    pub up: Box2,
    pub down: Box2,
}

/// 게임방법 창이 떠 있으면 그 네 타깃. 없으면 `None`.
///
/// ⛔ 제목 상자까지 확인한다. 삼각형 두 개만으로는 약하고, 「두 화살표 사이가 제목 상자」는
/// 이 창에만 있는 모양이다.
pub fn pager_header(px: &[u32], lw: i32, lh: i32) -> Option<PagerHeader> {
    if lw <= 0 || lh <= 0 || px.len() != (lw as usize) * (lh as usize) {
        return None;
    }
    let (ox, oy) = panel_origin(lw, lh)?;
    let at = |dx: i32, dy: i32, want: u32| -> bool {
        let (x, y) = (ox + dx, oy + dy);
        x >= 0 && x < lw && y >= 0 && y < lh && px[(y as usize) * (lw as usize) + x as usize] & 0x00ff_ffff == want
    };
    let mut runs: Vec<(i32, i32)> = Vec::new();
    let mut start: Option<i32> = None;
    for dx in 0..=PANEL_W {
        match (start, dx < PANEL_W && (0..PANEL_H).any(|dy| at(dx, dy, PAGER_TRI_INK))) {
            (None, true) => start = Some(dx),
            (Some(a), false) => {
                let w = dx - a;
                if (PAGER_TRI_W_MIN..=PAGER_TRI_W_MAX).contains(&w) {
                    runs.push((a, dx - 1));
                }
                start = None;
            }
            _ => {}
        }
    }
    let [(l0, l1), (r0, r1)] = runs[..] else { return None };
    if r0 <= l1 + 1 {
        return None;
    }
    let rows: Vec<i32> = (0..PANEL_H)
        .filter(|&dy| (l0..=l1).chain(r0..=r1).any(|dx| at(dx, dy, PAGER_TRI_INK)))
        .collect();
    let (&y0, &y1) = (rows.first()?, rows.last()?);
    // 두 화살표 사이가 제목 상자인가 — 그 가운데 줄로 본다.
    let mid = (y0 + y1) / 2;
    let span = r0 - l1 - 1;
    let title = (l1 + 1..r0).filter(|&dx| at(dx, mid, PAGER_TITLE_INK)).count() as i32;
    if span <= 0 || title * PAGER_TITLE_FILL_DEN < span * PAGER_TITLE_FILL_NUM {
        return None;
    }
    let top = (y0 - PAGER_HEADER_PAD).max(0);
    let bot = (y1 + PAGER_HEADER_PAD).min(PAGER_BODY_TOP - 1);
    let head = |dx: i32, w: i32| Box2 { dx, dy: top, w, h: bot - top + 1 };
    // 본문은 위/아래 절반으로 나눈다 — 게임이 스크롤에 쓰는 키가 UP/DOWN 뿐이라
    // 그것 말고 나눌 축이 없다.
    let split = (PAGER_BODY_TOP + PAGER_BODY_BOT) / 2;
    let (pl, pr) = ((l1 + 1 + 4).min(PANEL_W - 1), (r0 - 4).max(0));
    Some(PagerHeader {
        prev: head(0, pl),
        pick: head(pl, (pr - pl).max(1)),
        next: head(pr, PANEL_W - pr),
        up: Box2 { dx: 0, dy: PAGER_BODY_TOP, w: PANEL_W, h: split - PAGER_BODY_TOP + 1 },
        down: Box2 { dx: 0, dy: split + 1, w: PANEL_W, h: PAGER_BODY_BOT - split },
    })
}

// ── 환경설정(OPTION) 창 (80차-K) ────────────────────────────────────────────────
//
// 제보: *"인게임 -> 시스템 에서 환경설정을 들어갔을떄도 터치가 안되고있고 … 메인메뉴의
// 환경설정도 터치 안되고있어"*. 타이틀과 인게임이 **같은 창**이다.
//
// 실측(wie_probe): 일곱 줄, `dx 32..143` · `dy0 24` · 높이 14 · 간격 17. 안 선택된 줄은
// **`#cecac5` 단색**이고 선택된 줄은 어두운 체커(`#4a3929`/`#4a3d31`)다 — 80차-C 의 셸 SLOT
// 목록과 정확히 같은 모양이라 판정도 같다: **밝지 않은 줄이 커서**다.
// 선택된 줄의 값 `< N >` 은 흰 글자로 `<` dx 100..104 · 값 116..122 · `>` 133..137.
pub const OPTION_X0: i32 = 32;
pub const OPTION_W: i32 = 112;
pub const OPTION_ROWS: i32 = 7;
/// 안 선택된 줄의 단색.
pub const OPTION_ROW_INK: u32 = 0x00ce_cac5;
/// 프로브(496x240)에서 잰 값. **감지기는 이 값을 쓰지 않는다** — 아래 ⛔ 를 볼 것.
pub const OPTION_Y0: i32 = 24;
pub const OPTION_H: i32 = 14;
pub const OPTION_PITCH: i32 = 17;
/// 한 줄로 인정할 가로 잉크 수 (한 줄은 112 px 인데 글자가 그 위에 얹힌다 — 실측 최소 63).
pub const OPTION_RUN_MIN: i32 = 56;
/// 커서 줄로 인정할 어두운 픽셀 수와 그 기준(RGB 합). 선택 채움은 디더 `#4a3d31`/`#4a3929`
/// (합 184/172)이고 그 위에 흰 글자가 얹힌다 — 실측 최소 84.
pub const OPTION_DARK_MIN: i32 = 72;
pub const OPTION_DARK_SUM: u32 = 260;
/// 줄 높이와 간격의 허용 범위.
pub const OPTION_H_MIN: i32 = 12;
pub const OPTION_H_MAX: i32 = 16;
pub const OPTION_PITCH_MIN: i32 = 16;
pub const OPTION_PITCH_MAX: i32 = 20;
/// 커서 줄을 찾을 때 계산된 자리에서 이만큼 위아래를 본다.
pub const OPTION_SEEK: i32 = 2;
/// 값 `< N >` 이 그려지는 칸의 시작과, `<` / `>` 를 가르는 경계 (실측값 사이).
pub const OPTION_VALUE_X0: i32 = 94;
pub const OPTION_VALUE_SPLIT: i32 = 116;

/// 환경설정 창의 일곱 줄과 지금 커서가 앉은 줄. 없으면 `None`.
///
/// ⛔⛔⛔ **기하를 상수로 박지 말 것 — 픽셀에서 유도한다** (80차-O, 기기 실측). 80차-K 는
/// 프로브의 `dy0 24 · 높이 14 · 간격 17` 을 상수로 박았는데 **같은 창이 기기에서는
/// `dy0 19 · 높이 14 · 간격 18`** 이다(S9, 인게임 시스템 → 환경설정. 두 프레임의 커서 줄
/// 어두운 픽셀 수가 열넷 다 바이트 동일이라 샘플링 오차가 아니다). 상수판은 기기에서
/// **첫 줄 자리를 배경으로 읽고**(= 「어두운 줄」) 나머지 여섯이 우연히 밝아서 창을 찾긴
/// 했지만, 커서를 언제나 0 이라고 답했고 vc267 의 간격 검사가 들어가자 통째로 죽었다.
///
/// ⛔⛔ 그리고 줄 수만으로는 **인게임 6행 MENU 목록**과 안 갈린다(vc266 에 실제로 그것이
/// 출고돼 메뉴 탭이 통째로 죽었다 — 제보: *"가방에 포커스된채로 가방 한번더 터치하면
/// 가방창으로 들어가져야하는데 안들어가진다"*). MENU 는 줄 15 · 간격 **19** 라 고정 격자를
/// 얹으면 밴드 하나가 **두 줄에 걸쳐** 각각 절반씩 먹어 여섯이 전부 「밝다」로 읽힌다.
/// 지금은 밴드를 **찾아서** 세므로 MENU 는 밝은 밴드가 다섯(커서 하나는 어둡다)이라
/// 「여섯」에서 바로 떨어진다.
///
/// 판정: 밝은 밴드 **여섯** · 높이가 전부 같고 · 간격이 한 값 `p` 이며 · 커서 줄이 낀 곳만
/// `2p` (커서가 맨 위/맨 아래면 간격은 전부 `p` 이고 그 바깥 한 칸이 어둡다).
pub fn option_rows(px: &[u32], lw: i32, lh: i32) -> Option<(u8, Vec<Box2>)> {
    if lw <= 0 || lh <= 0 || px.len() != (lw as usize) * (lh as usize) {
        return None;
    }
    let (ox, oy) = panel_origin(lw, lh)?;
    let at = |dx: i32, dy: i32| -> Option<u32> {
        let (x, y) = (ox + dx, oy + dy);
        (x >= 0 && x < lw && y >= 0 && y < lh).then(|| px[(y as usize) * (lw as usize) + x as usize])
    };
    let lit = |dy: i32| {
        (OPTION_X0..OPTION_X0 + OPTION_W).filter(|&dx| at(dx, dy).is_some_and(|p| p & 0x00ff_ffff == OPTION_ROW_INK)).count() as i32
    };
    let dim = |dy: i32| {
        (OPTION_X0..OPTION_X0 + OPTION_W)
            .filter(|&dx| at(dx, dy).is_some_and(|p| ((p >> 16) & 0xff) + ((p >> 8) & 0xff) + (p & 0xff) < OPTION_DARK_SUM))
            .count() as i32
    };

    // 밝은 줄을 밴드로 모은다.
    let mut bands: Vec<(i32, i32)> = Vec::new();
    let mut open: Option<i32> = None;
    for dy in 0..=PANEL_H {
        match (dy < PANEL_H && lit(dy) >= OPTION_RUN_MIN, open) {
            (true, None) => open = Some(dy),
            (false, Some(top)) => {
                bands.push((top, dy - top));
                open = None;
            }
            _ => {}
        }
    }
    if bands.len() != OPTION_ROWS as usize - 1 {
        return None;
    }
    let h = bands[0].1;
    if !(OPTION_H_MIN..=OPTION_H_MAX).contains(&h) || bands.iter().any(|&(_, bh)| bh != h) {
        return None;
    }
    let diffs: Vec<i32> = bands.windows(2).map(|w| w[1].0 - w[0].0).collect();
    let pitch = *diffs.iter().min()?;
    if !(OPTION_PITCH_MIN..=OPTION_PITCH_MAX).contains(&pitch) || pitch <= h {
        return None;
    }
    // ⛔⛔ 간격을 **정확히** 비교하지 말 것 — 창 하나 안에서도 17 과 18 이 섞인다(실측 S9:
    // 19·36·54·72·90·108·126). 그래서 커서가 가운데 줄이면 빈 자리가 `2p` 가 아니라 `2p-1` 로
    // 나오고, vc269 1차 판이 그것 때문에 두 번째 줄부터 전부 거절했다(기기 로그
    // `option row 0 -> 1: DOWN x1` 뒤 `is not a row or a cell on this screen`).
    let mut holes: Vec<usize> = Vec::new();
    for (i, &d) in diffs.iter().enumerate() {
        if (d - pitch).abs() <= 1 {
            continue;
        }
        if (d - pitch * 2).abs() <= OPTION_SEEK + 1 {
            holes.push(i);
        } else {
            return None;
        }
    }
    if holes.len() > 1 {
        return None;
    }
    // 커서 줄은 어둡고, **줄 너비만큼만** 어둡다.
    // ⛔ 그 두 번째 조건이 밑단을 막는다 (실측): 게임의 푸터 띠도 높이 14 에 `dim` 이 92~112 라
    // 세로로만 보면 커서 줄과 구분되지 않는데, 그것은 패널을 **가로로 다 채운다**.
    let dark_at = |top: i32| {
        top >= 0
            && top + h <= PANEL_H
            && (top..top + h).all(|dy| {
                let edge = |dx: i32| at(dx, dy).is_some_and(|p| ((p >> 16) & 0xff) + ((p >> 8) & 0xff) + (p & 0xff) < OPTION_DARK_SUM);
                dim(dy) >= OPTION_DARK_MIN && !edge(OPTION_X0 - 2) && !edge(OPTION_X0 + OPTION_W + 1)
            })
    };
    // ⛔ 간격은 창 안에서도 한 값이 아니다 (실측 S9: 19·36·54·72·90·108·126 = 17 하나에 18 다섯).
    // 그래서 커서 줄은 **계산이 아니라 그 언저리에서 찾는다**.
    let find_dark = |want: i32| (-OPTION_SEEK..=OPTION_SEEK).map(|o| want + o).find(|&t| dark_at(t));

    let (cursor, dark_top) = match holes.first() {
        Some(&i) => (i + 1, find_dark(bands[i].0 + pitch)?),
        None => {
            // 커서가 맨 위이거나 맨 아래다 — 둘 중 정확히 하나만 어두워야 한다.
            match (find_dark(bands[0].0 - pitch), find_dark(bands[bands.len() - 1].0 + pitch)) {
                (Some(t), None) => (0, t),
                (None, Some(t)) => (OPTION_ROWS as usize - 1, t),
                _ => return None,
            }
        }
    };
    let mut tops: Vec<i32> = bands.iter().map(|&(t, _)| t).collect();
    tops.insert(cursor, dark_top);
    let boxes = tops.iter().map(|&t| Box2 { dx: OPTION_X0, dy: t, w: OPTION_W, h: pitch }).collect();
    Some((cursor as u8, boxes))
}

/// 제련창 **개수 스피너**의 삼각형 잉크 (74,49,33).
pub const SPIN_INK: u32 = 0x004a_3121;
/// 삼각형 밑변으로 인정할 런 길이. ★ 실측(66차-B, 아이템제련): ▲ 는 dy 36(1px)→39(x 114..120),
/// ▼ 는 dy 56(x 114..120)→59(1px) 로 각각 4행이고 밑변이 5~7px 다. ⛔ **긴 런은 배제해야
/// 한다** — 확인창의 선택된 `예` 배경이 같은 색이고 52px 짜리 런을 만든다.
pub const SPIN_RUN_MIN: i32 = 5;
pub const SPIN_RUN_MAX: i32 = 8;
/// 두 삼각형 사이의 세로 간격 (실측 17행: dy 39 → 56).
pub const SPIN_GAP_MIN: i32 = 12;
pub const SPIN_GAP_MAX: i32 = 24;
/// 삼각형은 6px 밖에 안 되므로 엄지를 위해 이만큼 키운다. 세로는 숫자 칸(dy 44..50)을
/// 건드리지 않는 만큼만.
pub const SPIN_PAD_X: i32 = 12;
pub const SPIN_PAD_Y: i32 = 5;

/// 제련창의 개수 스피너 ▲/▼ 를 찾아 그 히트 사각형 둘을 돌려준다. 없으면 `None`.
///
/// ★★ 이 화면을 **찾아내는 것 자체가 안전장치**다 (66차-B). 아이템제련 화면은 창 13 인데
/// 공용 위젯 바이트가 앞 목록의 낡은 값(`rows=3`, `paint=12`)을 들고 있어서, 행 목록으로
/// 오판하면 탭이 곧 `제련하기` OK 가 된다. 스피너가 보이면 **패널의 다른 곳은 우리 것이
/// 아니다** — 화살표 둘만 살아 있고 나머지는 무동작이다.
///
/// 삼각형은 UP/DOWN 키다(실측: D-패드 UP 을 누르면 숫자 칸 x 119..125 만 바뀐다). 그래서
/// 여기서 나온 사각형의 탭은 **키 하나**로 끝나고, 게스트 메모리를 건드리지 않는다.
pub fn spinner_arrows(px: &[u32], lw: i32, lh: i32) -> Option<(Box2, Box2)> {
    if lw <= 0 || lh <= 0 || px.len() != (lw as usize) * (lh as usize) {
        return None;
    }
    let (ox, oy) = panel_origin(lw, lh)?;
    let at = |dx: i32, dy: i32| -> bool {
        let (x, y) = (ox + dx, oy + dy);
        x >= 0 && x < lw && y >= 0 && y < lh && px[(y as usize) * (lw as usize) + x as usize] & 0x00ff_ffff == SPIN_INK
    };
    // 밑변 후보: 길이 5..=8 인 런. 긴 런(확인창 배경)은 여기서 걸러진다.
    let mut base: Vec<(i32, i32, i32)> = Vec::new(); // (dy, x0, x1)
    for dy in 0..PANEL_H {
        let mut start: Option<i32> = None;
        for dx in 0..=PANEL_W {
            let on = dx < PANEL_W && at(dx, dy);
            match (start, on) {
                (None, true) => start = Some(dx),
                (Some(a), false) => {
                    let len = dx - a;
                    if (SPIN_RUN_MIN..=SPIN_RUN_MAX).contains(&len) {
                        base.push((dy, a, dx - 1));
                    }
                    start = None;
                }
                _ => {}
            }
        }
    }
    // 위/아래 한 쌍: x 중심이 같고 세로로 SPIN_GAP 만큼 떨어진 둘.
    for &(uy, ua, ub) in &base {
        for &(dy2, da, db) in &base {
            let gap = dy2 - uy;
            if !(SPIN_GAP_MIN..=SPIN_GAP_MAX).contains(&gap) {
                continue;
            }
            if ((ua + ub) - (da + db)).abs() > 4 {
                continue;
            }
            let grow = |dy: i32, a: i32, b: i32| Box2 {
                dx: (a - SPIN_PAD_X).max(0),
                dy: (dy - SPIN_PAD_Y).max(0),
                w: (b + 1 + SPIN_PAD_X).min(PANEL_W) - (a - SPIN_PAD_X).max(0),
                h: SPIN_PAD_Y * 2 + 1,
            };
            // ▲ 는 밑변이 아래쪽이라 그 위로, ▼ 는 밑변이 위쪽이라 그 아래로 자란다.
            return Some((grow(uy - 2, ua, ub), grow(dy2 + 2, da, db)));
        }
    }
    None
}

/// 게임이 아이템 칸의 **빈 틀**을 그리는 잉크 (49,20,8).
pub const SLOT_INK: u32 = 0x0031_1408;
/// 한 칸 틀의 가로 테두리 길이 허용 범위 (실측: 가방 15~17, 일반조합 15).
/// 칸 틀의 **위 테두리** 런 길이 (실측: 가방·일반조합 모두 정확히 15).
pub const SLOT_BORDER: i32 = 15;

/// 한 행에서 인정하는 **최대 칸 간격**. 실측된 것은 둘뿐이다 — 가방 19 · 일반조합 29,
/// 칸은 둘 다 17.
///
/// ★★ 왜 상한이 필요한가 (79차-E 리뷰): 아래 `border_row` 는 간격을 **런 사이 최소 간격**
/// 으로 잡는데, 런은 게임이 그린 **빈 칸 틀**이라 아이템이 든 칸에는 아예 없다. 빈 칸이
/// 하나 걸러 하나(예: 0·2·4·6번 칸)만 남으면 모든 간격이 `2 * 19 = 38` 이 되고, "나머지는
/// 그 배수" 검사는 **전부 통과한다** — 런이 둘뿐인 행에서는 그 검사가 아예 공허하다.
/// 그러면 격자가 4열로 읽혀서 실제 2번 칸 탭이 인덱스 1 로 나간다. 워커의 교차검증은
/// 「그려진 커서 인덱스 == 라이브 커서 바이트」만 보므로 커서가 0번에 앉아 있으면 통과한다.
///
/// 칸 하나가 통째로 들어가는 틈(간격 >= 칸 크기 x 2)은 어떤 실측 배치에도 없으므로, 그건
/// 배수를 최소로 오인한 것이다 — 추측하지 말고 그 행을 버린다(그러면 격자를 못 읽고,
/// `menu_tap_action` 이 `cell_cursor` 를 보고 아무것도 안 한다).
pub const SLOT_PITCH_MAX: i32 = INVEN.w * 2;

/// 지금 화면의 **아이템 격자**를 픽셀에서 유도한다 — `(격자, 그 격자에서 노란 테두리가 있는
/// 칸의 인덱스)`.
///
/// ★★ 왜 유도해야 하는가 (66차-B, 제보: *"일반 조합창에서 아이템 클릭했는데 엉뚱한 아이템이
/// 포커스돼 … 5번째 항목 선택하면 … 가운데 창도 버그처럼 나와"*): 아이템 격자가 창마다 다르다.
///
/// ```text
///   가방(인벤·상점 판매·제련 재료)  8칸 x 2행, 간격 19, 칸 17
///   일반조합                        5칸 x 1행, 간격 **29**, 칸 17   <- 같은 칸 크기!
/// ```
///
/// 칸 크기가 같으니 노란 테두리만으로는 못 가리고, 가방 기하를 그대로 쓰면 5번째 칸 탭이
/// 인덱스 14 로 읽혀 **범위 밖 커서**가 된다 — 그러면 게임은 선택 테두리를 아예 안 그리고
/// 엉뚱한 아이템의 상세를 그린다(제보의 "버그처럼 나와"가 정확히 그것이다).
///
/// 그래서 **게임이 그린 칸 틀**에서 x 시작·간격·칸 수를 읽고, 세로도 같은 방식으로 행 수를
/// 읽는다. 돌려주는 인덱스는 **픽셀만으로 계산한 값**이므로, 워커가 그것을 게임의 커서
/// 바이트와 대조해서 어긋나면 아무것도 하지 않을 수 있다(그 교차검증이 안전장치다).
pub fn item_grid(px: &[u32], lw: i32, lh: i32) -> Option<(Grid, u8)> {
    let (bx, by) = cell_box(px, lw, lh)?;
    if lw <= 0 || lh <= 0 || px.len() != (lw as usize) * (lh as usize) {
        return None;
    }
    let (ox, oy) = panel_origin(lw, lh)?;
    let at = |dx: i32, dy: i32| -> bool {
        let (x, y) = (ox + dx, oy + dy);
        x >= 0 && x < lw && y >= 0 && y < lh && px[(y as usize) * (lw as usize) + x as usize] & 0x00ff_ffff == SLOT_INK
    };
    // ★ 칸 틀의 **위 테두리**만 쓴다: 정확히 SLOT_BORDER 길이의 런들이 균일한 간격으로 늘어선
    //   행이다. 실측(66차-B):
    //     가방      dy 18 [33 52 71 90 109 128 147] · dy 37 [14 33 … 147]  간격 19
    //     일반조합  dy 18 [22 51 80 109 138]                               간격 29
    //   칸 내부(길이 17)와 아래 테두리(길이 10)는 아이템 그림 때문에 지저분하므로 안 쓴다.
    //   ⛔ 커서가 앉은 칸은 노란 테두리가 그 틀을 덮어서 **런이 하나 빠진다**(가방 첫 행이
    //   8개가 아니라 7개인 이유가 그것이다) — 그래서 칸 수는 런 개수가 아니라 **처음과 끝
    //   좌표 + 커서 테두리**에서 계산한다.
    let border_row = |dy: i32| -> Option<(i32, Vec<i32>)> {
        let mut starts = Vec::new();
        let mut start: Option<i32> = None;
        for dx in 0..=PANEL_W {
            let on = dx < PANEL_W && at(dx, dy);
            match (start, on) {
                (None, true) => start = Some(dx),
                (Some(a), false) => {
                    if dx - a == SLOT_BORDER {
                        starts.push(a);
                    }
                    start = None;
                }
                _ => {}
            }
        }
        if starts.len() < 2 {
            return None;
        }
        // ★ 간격은 **가장 좁은 간격**이고, 나머지는 그 배수여야 한다. 커서가 앉은 칸은 틀이
        // 노란 테두리에 덮여 런이 빠지므로 그 자리에서 간격이 2배로 벌어진다 — 실측(일반조합,
        // 커서 3): 시작들이 [22, 51, 80, 138] 로 마지막 간격만 58 이었다. 균일함을 요구하면
        // 바로 그 창을 놓친다.
        let pitch = starts.windows(2).map(|w| w[1] - w[0]).min()?;
        if !(CELL_BOX_MIN..=SLOT_PITCH_MAX).contains(&pitch) {
            return None;
        }
        starts
            .windows(2)
            .all(|w| {
                let gap = w[1] - w[0];
                gap % pitch <= 1 || pitch - gap % pitch <= 1
            })
            .then_some((pitch, starts))
    };
    // 위 테두리 행들 -> 세로 띠. 서로 3px 안쪽이면 같은 띠다.
    let mut bands: Vec<(i32, i32, Vec<i32>)> = Vec::new(); // (dy, pitch, starts)
    for dy in 0..PANEL_H {
        // ⛔ let-chain 을 쓰지 말 것 — 이 파일은 `menutouch_selftest.rs` 가 edition 2021 로
        // 컴파일한다(그 테스트가 이 실제 소스를 include! 한다).
        if let Some((pitch, starts)) = border_row(dy) {
            // ★ 다음 행의 위 테두리는 최소 한 칸(17) 아래다. 그보다 가까운 것은 **같은 행의
            // 아래 테두리**이므로 행으로 세면 안 된다 — 실측에서 그것 때문에 가방이 3행으로
            // 읽히고 세로 간격이 16 으로 나왔다.
            if bands.last().is_none_or(|(last, _, _)| dy - last >= INVEN.pitch_y - 2) {
                bands.push((dy, pitch, starts));
            }
        }
    }
    // ★★★ 가방이 꽉 차면 유도할 잉크가 **하나도 없다** (79차-E, 기기 제보: *"가방 들어갔을때
    // 템 선택(포커스 이동)이 터치가 안 된다"*). 이 함수는 게임이 그린 **빈 칸 틀**에서 격자를
    // 읽는데, 빈 칸이 없으면 틀도 없다 — 그리고 한 행에 런이 둘은 있어야 하므로 **빈 칸 1개도
    // 부족하다**. LV70 세이브처럼 가방이 꽉 찬 플레이어에게는 인벤 탭이 통째로 죽어 있었다.
    //
    // 남는 신호는 커서 테두리 하나뿐이고, 그것이 **알려진 가방 격자의 칸 원점과 정확히 일치**
    // 하면 그 화면은 가방이다(간격 19 · 칸 17 · 고정 위치, 패널이 176x160 중앙정렬이라 해상도
    // 무관). 일반조합(간격 29)은 원점이 하나(dx 108)만 겹치는데 그 자리의 인벤 인덱스는 5 이고
    // 일반조합 커서는 3 이라, 워커의 교차검증(픽셀 인덱스 == 라이브 커서 바이트)이 거른다.
    // ⛔ 그 교차검증이 이 폴백의 안전장치다 — `item_grid_action` 에서 빼지 말 것.
    if bands.is_empty() {
        return INVEN.origin_index(bx, by).map(|idx| (INVEN, idx));
    }
    let (first_dy, pitch_x, _) = bands.first()?.clone();
    if bands.iter().any(|(_, p, _)| *p != pitch_x) {
        return None; // 창 안에 서로 다른 격자가 둘 — 우리가 본 적 없는 배치다
    }
    // 칸 좌표는 테두리보다 1px 왼쪽/위다 (실측: 테두리 33 <-> 커서 테두리 32).
    let all: Vec<i32> = bands.iter().flat_map(|(_, _, st)| st.iter().map(|s| s - 1)).collect();
    let x0 = all.iter().copied().chain([bx]).min()?;
    let last_x = all.iter().copied().chain([bx]).max()?;
    if (bx - x0) % pitch_x != 0 || (last_x - x0) % pitch_x != 0 {
        return None; // 커서가 이 격자에 안 맞는다 — 우리 것이 아니다
    }
    let cols = (last_x - x0) / pitch_x + 1;
    let y0 = first_dy.min(by);
    let pitch_y = if bands.len() >= 2 { bands[1].0 - bands[0].0 } else { INVEN.pitch_y };
    if pitch_y <= 0 || (by - y0) % pitch_y != 0 {
        return None;
    }
    let rows = ((bands.last()?.0.max(by) - y0) / pitch_y) + 1;
    let (col, row) = ((bx - x0) / pitch_x, (by - y0) / pitch_y);
    if col < 0 || col >= cols || row < 0 || row >= rows || cols * rows > 64 {
        return None;
    }
    let grid = Grid {
        x0,
        y0,
        // 칸 크기는 실측 표(`INVEN`)의 것이다 — 창이 달라도 칸은 17x17 이었다.
        w: INVEN.w,
        h: INVEN.h,
        pitch_x,
        pitch_y,
        cols,
        rows,
        pad_x: 0,
        pad_y: 0,
    };
    Some((grid, (row * cols + col) as u8))
}

/// 게임 자신의 가방 수 (0..7). 셀 커서는 **페이지로 인덱싱되는 배열**이라
/// (`CHAR+0x9c72 + page`, `items::cash_use_pending` 이 읽는 그 두 바이트) 페이지가
/// 이 범위를 넘으면 레코드의 **다른 바이트**를 쓰게 된다 — 그래서 클램프가 아니라 거절이다.
/// ★ 기기 실측(66차-B): 페이지 0·1·2 에서 화살표가 각각 `0x9c72`·`0x9c73`·`0x9c74` 를 움직였다.
/// 그 배열은 6번째 가방부터 **장비 화면의 커서 3바이트(0x9c77~79)와 겹친다** — 게임 자신의
/// 배치가 그렇고(화면 전환 때 각자 초기화한다), 우리는 게임이 쓰는 바로 그 바이트를 쓴다.
pub const PAGES: u8 = 8;

/// 패널 상대 사각형 하나 (배지·스피너 화살표·확인창 버튼처럼 격자가 아닌 타깃용).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Box2 {
    pub dx: i32,
    pub dy: i32,
    pub w: i32,
    pub h: i32,
}

impl Box2 {
    pub fn contains(&self, dx: i32, dy: i32) -> bool {
        dx >= self.dx && dx < self.dx + self.w && dy >= self.dy && dy < self.dy + self.h
    }
}

/// ★ 136차 (2026-09-25) — 「정렬」 버튼. 인벤토리·싱글가방·창고 패널의 **돈 줄**(동전 + 소지금,
/// 프로브 실측 dy 115..127 · 동전 dx 145..151 · 숫자 163..)의 **왼쪽 빈 구간**에 호스트가 게임
/// 폰트로 「정렬」을 그리고, 이 상자가 그 탭을 받는다. 세로는 줄의 흰 테두리(113·128)까지 넉넉히.
/// 사용자: *"창고, 이름, 효과 밑에 돈 표시되는영역의 좌측끝에 정렬"*.
pub const SORT_BUTTON: Box2 = Box2 { dx: 0, dy: 111, w: 44, h: 20 };
/// 돈 줄의 세로 범위 (패널 dy, 끝 제외) — 라벨을 그 가운데에 놓는다.
pub const MONEY_STRIP_DY: (i32, i32) = (115, 128);
/// 창고(NETWORK) 패널만 돈 줄 왼쪽에 게임이 **`✱`** 힌트를 그린다 (`*` 키 = 골드 입금창
/// 「금액을 설정하세요」, 프로브 실측 dx 17..23). 터치 HUD 에 `*` 가 없어진 뒤 닿을 수 없는
/// 기능이라 136차에 걷어냈다(사용자 결정): 호스트가 배경색으로 덧칠하고 그 자리에 정렬을 둔다.
pub const WH_STAR_HINT: Box2 = Box2 { dx: 16, dy: 115, w: 9, h: 13 };
/// 온라인 패널 돈 줄의 단색 배경 (프로브 실측 `#c5b6a5`) — `✱` 덧칠에 쓴다.
pub const ONLINE_STRIP_BG: u32 = 0x00c5_b6a5;

/// 패널 상대 탭이 「정렬」 버튼 위인가.
pub fn sort_button_hit(dx: i32, dy: i32) -> bool {
    SORT_BUTTON.contains(dx, dy)
}

/// 모듈이 **자기 힘으로 그리는 `예 / 아니요` 확인창**의 선택된 버튼 잉크 (90,28,0), 그리고
/// 안 선택된 쪽의 배경 (189,178,173).
///
/// ★★ 이 창은 공용 위젯이 아니다 (66차-B, "착용중인 장비입니다. 계속하시겠습니까?"): RIGHT 를
/// 눌러도 **레코드의 어떤 바이트도 안 움직이는데** 화면에서는 선택이 옮겨간다(변경 픽셀 1,040개,
/// dy 94..106). 즉 선택 상태가 제련 모듈 안에 있다 — 그래서 **픽셀이 유일한 창구**다.
/// 실측: 예 = x 29..68, 아니요 = x 105..144, 둘 다 40x13, 밴드 dy 94..106.
/// 그 잉크는 다른 29장의 실측 프레임 어디에도 없다.
pub const CONFIRM2_SEL_INK: u32 = 0x005a_1c00;
pub const CONFIRM2_BG_INK: u32 = 0x00bd_b2ad;
/// 두 버튼의 폭/높이 허용 범위 (실측 40x13).
pub const CONFIRM2_W_MIN: i32 = 30;
pub const CONFIRM2_W_MAX: i32 = 50;
pub const CONFIRM2_H_MIN: i32 = 10;
pub const CONFIRM2_H_MAX: i32 = 18;
/// 짝의 좌우 대칭 상수: `왼쪽 x0 + 오른쪽 x1` (실측 29 + 144). 반대쪽 버튼을 이걸로 접어서
/// 구하고, **그 자리가 정말 안 선택된 배경인지 확인한 뒤에만** 인정한다.
pub const CONFIRM2_SPAN: i32 = 173;

/// 모듈이 직접 그린 `예/아니요` 확인창의 두 버튼 — `(선택된 것, 나머지)`. 없으면 `None`.
///
/// 선택 상태는 메모리에 없으므로 **여기서 나온 사각형이 곧 답**이다: 선택된 쪽 탭 = OK,
/// 나머지 쪽 탭 = 그쪽으로 가는 화살표. 게스트 메모리를 한 바이트도 안 건드린다.
pub fn confirm_pair(px: &[u32], lw: i32, lh: i32) -> Option<(Box2, Box2)> {
    if lw <= 0 || lh <= 0 || px.len() != (lw as usize) * (lh as usize) {
        return None;
    }
    let (ox, oy) = panel_origin(lw, lh)?;
    let ink = |dx: i32, dy: i32| -> Option<u32> {
        let (x, y) = (ox + dx, oy + dy);
        (x >= 0 && x < lw && y >= 0 && y < lh).then(|| px[(y as usize) * (lw as usize) + x as usize] & 0x00ff_ffff)
    };
    // 1) 각 행에서 선택 잉크가 걸쳐 있는 **범위**(첫 픽셀..끝 픽셀). ⛔ "가장 긴 런"으로
    //    보면 안 된다 — 버튼 위에 `예` 글자가 얹혀 있어서 박스가 조각난다(실측: dy 100 의
    //    런은 29..43 / 45..47 / 49 / 51 / 53..68 이고 가장 긴 것이 16px 밖에 안 된다).
    //    범위와 **채움 비율**로 보면 그 글자가 문제가 되지 않는다(같은 행 채움 90%).
    let span = |dy: i32| -> Option<(i32, i32)> {
        let mut first = None;
        let mut last = 0;
        let mut fill = 0;
        for dx in 0..PANEL_W {
            if ink(dx, dy) == Some(CONFIRM2_SEL_INK) {
                first = first.or(Some(dx));
                last = dx;
                fill += 1;
            }
        }
        let a = first?;
        let w = last - a + 1;
        ((CONFIRM2_W_MIN..=CONFIRM2_W_MAX).contains(&w) && fill * 2 >= w).then_some((a, last))
    };
    // 2) 같은 x 구간이 이어지는 행들을 하나의 박스로 묶는다.
    let mut cand: Vec<(i32, i32, i32, i32)> = Vec::new(); // (dy0, dy1, x0, x1)
    for dy in 0..PANEL_H {
        match span(dy) {
            Some((a, b)) => match cand.last_mut() {
                Some(last) if last.1 == dy - 1 && (last.2 - a).abs() <= 1 && (last.3 - b).abs() <= 1 => last.1 = dy,
                _ => cand.push((dy, dy, a, b)),
            },
            None => {}
        }
    }
    // 3) 높이가 맞는 박스의 좌우 대칭 자리가 **정말 안 선택된 배경**이어야 한다. 한 픽셀로
    //    확인하면 안 된다 — 그 자리에는 반대쪽 버튼의 글자가 있다(실측: dx 124 는 `아니요`의
    //    글리프였고 그 때문에 첫 판이 아무것도 못 찾았다). 그래서 면적 비율로 본다.
    for &(dy0, dy1, x0, x1) in &cand {
        let h = dy1 - dy0 + 1;
        if !(CONFIRM2_H_MIN..=CONFIRM2_H_MAX).contains(&h) {
            continue;
        }
        let (o0, o1) = (CONFIRM2_SPAN - x1, CONFIRM2_SPAN - x0);
        if o0 < 0 || o1 >= PANEL_W {
            continue;
        }
        let mut bg = 0;
        let mut seen = 0;
        for dy in dy0..=dy1 {
            for dx in o0..=o1 {
                seen += 1;
                if ink(dx, dy) == Some(CONFIRM2_BG_INK) {
                    bg += 1;
                }
            }
        }
        if seen > 0 && bg * 5 >= seen * 2 {
            let boxed = |a: i32, b: i32| Box2 { dx: a, dy: dy0, w: b - a + 1, h };
            return Some((boxed(x0, x1), boxed(o0, o1)));
        }
    }
    // ★★ 80차-G — 페인터가 셋이라 폴백도 셋이다: 제련(위) → 모달 페인터 → 공용 위젯.
    // 순서는 좁은 것부터다. 앞의 둘은 40x16 짜리 모달 상자이고 마지막은 52x14 짜리 위젯이라
    // 크기가 겹치지 않는다.
    confirm_pair_shell(px, lw, lh).or_else(|| confirm_pair_widget(px, lw, lh))
}

/// 셸 확인창(「네트워크 창고에 접속하시겠습니까?」)의 **선택된** 버튼: 통째로 칠해진 채움.
///
/// ★★ 80차-B 실측 (S9, vc246, 오프라인). 위의 제련 확인창과 **기하는 같고 팔레트만 다르다** —
/// 둘 다 40px 폭에 같은 밴드다. 실측: 선택 = dx 25..64 / dy 91..106 (40x16) 채움,
/// 안 선택 = dx 111..150 / dy 91..105 (40x15) **외곽선**.
/// 이 두 색은 79차-C 가 동작 팝업에서 잰 팔레트(몸통 `0xbdb2ad` · 행 `0x7b5952` ·
/// 선택 `0x4a3121`)와 같은 계열이다 — 게임이 모달 크롬을 돌려쓴다.
pub const CONFIRM3_SEL_INK: u32 = 0x004a_3121;
/// 같은 창에서 **안 선택된** 버튼: 채움이 없고 이 색 외곽선만 있다.
/// ⛔ 이 색으로 박스를 세울 때 "행마다 40px" 을 요구하면 안 된다 — 가득 찬 행은 위/아래 두
/// 줄뿐이고 가운데는 양옆 2px 씩이다. 패널 전체의 **바운딩 박스**로 잡는다.
pub const CONFIRM3_OUT_INK: u32 = 0x009c_7d73;

/// 셸 확인창의 두 버튼 — `(선택된 것, 나머지)`. 없으면 `None`.
///
/// 제련판(위)과 갈라 둔 이유는 알고리즘이 다르기 때문이다: 저쪽은 한 잉크를 찾아 [`CONFIRM2_SPAN`]
/// 으로 접어 반대쪽을 구하지만, 여기는 **두 버튼이 서로 다른 잉크로 직접 그려져 있어서** 접을
/// 필요가 없다. 접지 않는 편이 낫기도 하다 — 이 창의 실측 span 은 25+150 = 175 로 제련의 173 과
/// 다르고, 상수 하나를 두 창이 나눠 쓰면 그 2px 이 조용한 오검출이 된다.
///
/// 플레이어 제보 2026-09-05 (vc244): *"confirm류 팝업들에서 터치안먹고있고"*. 게이트를 고친 뒤
/// 기기 로그가 `menu tap: panel (43,99) -> no live character` 에서
/// `no ... targets` 로 바뀌며 **감지기가 이 창을 못 알아본다**는 것이 드러났다.
fn confirm_pair_shell(px: &[u32], lw: i32, lh: i32) -> Option<(Box2, Box2)> {
    let (ox, oy) = panel_origin(lw, lh)?;
    let ink = |dx: i32, dy: i32| -> Option<u32> {
        let (x, y) = (ox + dx, oy + dy);
        (x >= 0 && x < lw && y >= 0 && y < lh).then(|| px[(y as usize) * (lw as usize) + x as usize] & 0x00ff_ffff)
    };
    // 패널 전체에서 그 잉크의 바운딩 박스. 다른 화면에 같은 색이 흩어져 있으면 박스가 커지고
    // 아래 치수 검사에서 떨어진다 — fail closed.
    let bbox = |want: u32| -> Option<Box2> {
        let (mut x0, mut y0, mut x1, mut y1) = (PANEL_W, PANEL_H, -1, -1);
        let mut n = 0;
        for dy in 0..PANEL_H {
            for dx in 0..PANEL_W {
                if ink(dx, dy) == Some(want) {
                    x0 = x0.min(dx);
                    y0 = y0.min(dy);
                    x1 = x1.max(dx);
                    y1 = y1.max(dy);
                    n += 1;
                }
            }
        }
        (n > 0).then(|| Box2 { dx: x0, dy: y0, w: x1 - x0 + 1, h: y1 - y0 + 1 })
    };
    // ★★★ 80차-N — **외곽선을 먼저 잡고 접는다**, 채움을 먼저 잡지 않는다.
    //
    // 예전 판은 [`CONFIRM3_SEL_INK`] 의 **패널 전체 바운딩 박스**를 선택된 버튼으로 썼는데,
    // 그 색은 확인창 말고도 쓰인다: 새 캐릭터 생성의 「이대로 진행합니까?」 창에서는 화면
    // 크롬에도 있어서 박스가 dx 10..161 / dy 0..122 로 부풀어 크기 검사에서 떨어졌다
    // (프로브 실측 — 그 확인창이 감지기 셋 어디에도 안 걸린 이유가 이것이다).
    // 외곽선 쪽은 그 화면에서도 깨끗하다(dx 114..152 / dy 107..121).
    // ⇒ 외곽선으로 짝을 찾고, 패널 중앙으로 접은 자리가 **정말 칠해져 있는지** 확인한다
    //   (실측 채움 0.93 · 셸 확인창 둘도 같은 방식으로 성립한다).
    let other = bbox(CONFIRM3_OUT_INK)?;
    let ok = |b: &Box2| {
        (CONFIRM2_W_MIN..=CONFIRM2_W_MAX).contains(&b.w) && (CONFIRM2_H_MIN..=CONFIRM2_H_MAX).contains(&b.h)
    };
    if !ok(&other) {
        return None;
    }
    let sel = Box2 {
        dx: PANEL_W - 1 - (other.dx + other.w - 1),
        dy: other.dy,
        w: other.w,
        h: other.h,
    };
    if sel.dx < 0 || sel.dx + sel.w > PANEL_W {
        return None;
    }
    let filled = (sel.dy..sel.dy + sel.h)
        .flat_map(|dy| (sel.dx..sel.dx + sel.w).map(move |dx| (dx, dy)))
        .filter(|&(dx, dy)| ink(dx, dy) == Some(CONFIRM3_SEL_INK))
        .count() as i32;
    if filled * CONFIRM_FILL_DEN < sel.w * sel.h * CONFIRM_FILL_NUM {
        return None;
    }
    // ⛔ …and the filled thing must be a BUTTON, not a wider bar that merely contains one.
    // The action popup's selected row is the same ink at 62 px (79차-C), so measure the run
    // the mirror actually sits in — band-local, because the global bbox is what the
    // outline-first rewrite exists to stop trusting.
    let row = sel.dy + sel.h / 2;
    let mut lo = sel.dx;
    while lo > 0 && ink(lo - 1, row) == Some(CONFIRM3_SEL_INK) {
        lo -= 1;
    }
    let mut hi = sel.dx + sel.w - 1;
    while hi + 1 < PANEL_W && ink(hi + 1, row) == Some(CONFIRM3_SEL_INK) {
        hi += 1;
    }
    if hi - lo + 1 > CONFIRM2_W_MAX {
        return None;
    }
    // ⛔⛔ 80차-G 가 여기에 「외곽선이 없으면 패널 중앙으로 접는다」 폴백을 넣었다가 **되돌렸다**.
    // 접기 자체는 실측으로 맞다(셸 선택 dx 25..64 → 111..150, 실측 외곽선 111..149) — 문제는
    // 그러면 **짝 없는 채움 상자 하나가 확인창이 된다**는 것이고, `a_lone_confirm_ink_blob_is_
    // not_a_confirm_window` 가 지키는 것이 정확히 그 성질이다. 같은 색을 쓰는 선택된 행이
    // 화면에 널려 있다(동작 팝업 79차-C). 폴백이 필요했던 인게임 확인창은 이 페인터가 아니라
    // **공용 위젯**이었고, [`confirm_pair_widget`] 이 위치·크기까지 정확히 안다.
    // 나란히 놓인 두 버튼이어야 한다: 가로로 안 겹치고, 세로로는 거의 같은 줄.
    let apart = sel.dx + sel.w <= other.dx || other.dx + other.w <= sel.dx;
    let aligned = (sel.dy - other.dy).abs() <= 2;
    (apart && aligned).then_some((sel, other))
}

/// 게임이 패널 밑단에 그리는 **`OK : …` 프롬프트**의 주황 잉크 (222,154,25). 흰 `OK :` 뒤에
/// 오는 그 화면의 동작 이름(`선택` · `제련하기` …)이 이 색이다.
///
/// ★ 이것이 "이 밑단이 버튼인가"의 유일한 신호다 (66차-B 실측). 흰색만으로는 못 가린다 —
/// 패널 프레임 선이 흰색이라 프롬프트가 없는 화면(가방창 235px · 퀘스트 378px · SYSTEM 179px)
/// 에도 흰 픽셀이 그만큼 있다. 반면 이 주황은 프롬프트가 있는 화면에만 나온다
/// (제련 재료창 66px · 제련 스피너 134px, 나머지 0).
pub const OK_PROMPT_INK: u32 = 0x00de_9a19;
/// 프롬프트를 찾을 밑단의 시작 dy (실측: 흰 `OK :` 글자가 dy 150..156).
pub const OK_PROMPT_TOP: i32 = 140;
/// 이만큼은 있어야 프롬프트로 본다 (실측 66~134px).
pub const OK_PROMPT_MIN_INK: i32 = 20;
/// 글자 위/아래로 이만큼 키운다 — 밑단은 다른 타깃이 없어서 넉넉히 줘도 안전하다.
pub const OK_PROMPT_PAD: i32 = 4;

/// 패널 밑단의 `OK : …` 프롬프트를 찾아 그 히트 사각형을 돌려준다. 없으면 `None`.
///
/// 플레이어 요청(2026-08-23): *"밑에 OK:선택, OK:제련하기 버튼 눌렀을때 OK버튼처럼 동작해야
/// 하는데 그거 안돼"*. 게임이 스스로 "OK 를 누르라"고 적어 둔 자리이므로, 63차의 컷신
/// `# SKIP` 과 같은 성질의 타깃이다 — 게스트 RE 0, 메모리 쓰기 0, 보내는 키는 그 글자 그대로.
pub fn ok_prompt(px: &[u32], lw: i32, lh: i32) -> Option<Box2> {
    if lw <= 0 || lh <= 0 || px.len() != (lw as usize) * (lh as usize) {
        return None;
    }
    let (ox, oy) = panel_origin(lw, lh)?;
    let mut ink = 0;
    let (mut top, mut bot) = (PANEL_H, -1);
    for dy in OK_PROMPT_TOP..PANEL_H {
        let y = oy + dy;
        if y < 0 || y >= lh {
            continue;
        }
        let mut row = 0;
        for dx in 0..PANEL_W {
            let x = ox + dx;
            if x >= 0 && x < lw && px[(y as usize) * (lw as usize) + x as usize] & 0x00ff_ffff == OK_PROMPT_INK {
                row += 1;
            }
        }
        if row >= 2 {
            ink += row;
            top = top.min(dy);
            bot = bot.max(dy);
        }
    }
    if ink < OK_PROMPT_MIN_INK || bot < top {
        return None;
    }
    // ★ 80차-F — 아래는 언제나 밑단 밴드의 끝(패널 밖 [`FOOTER_OVERHANG`] 포함)이다. 잉크로
    // 자르던 예전 판은 스캔이 `PANEL_H` 에서 멈추므로 글자의 마지막 두 줄을 영영 못 봤다.
    let dy = (top - OK_PROMPT_PAD).clamp(0, FOOTER_BOX_TOP);
    let mut b = Box2 {
        dx: 0,
        dy,
        w: PANEL_W,
        h: FOOTER_BOX_BOT - dy,
    };
    // ★★ 같은 밑단에 `# : …` 이 있으면 그 칸은 내 것이 아니다 (80차-D). 게임시작 슬롯 목록의
    // 밑단이 정확히 그 모양이고(`OK : 이어하기` + `# : 초기화`), 거기서는 이전 캐릭터가 아직
    // resolve 되므로 `stats.level > 0` 게이트가 이 감지기를 **막아 주지 않는다** — 즉 밴드
    // 전체를 돌려주면 「초기화」를 눌렀을 때 OK(이어하기)가 나간다.
    // 호출 순서(`hash_prompt` 를 먼저 시험한다)만으로도 막히지만, 두 상자가 **구조적으로**
    // 안 겹치는 편이 셀프테스트로 못 박을 수 있는 불변식이다.
    if let Some(h) = hash_prompt(px, lw, lh) {
        if h.dx > 0 {
            b.w = h.dx;
        } else {
            b.dx = h.dx + h.w;
            b.w = PANEL_W - b.dx;
        }
    }
    (b.w > 0).then_some(b)
}

/// 게임이 밑단에 그리는 **`# : …` 프롬프트**의 흰 잉크. 토큰(`#` · `OK` · `CLR`)과 콜론,
/// 그리고 흰 동작 이름(`이어하기` · `메인메뉴`)이 이 색이고, 주황 동작 이름은
/// [`OK_PROMPT_INK`] 다 — 밑단에 오는 잉크는 그 둘과 셸 스프라이트의 크림 계열뿐이다.
pub const FOOTER_INK_WHITE: u32 = 0x00ff_ffff;
/// 밑단 프롬프트를 찾을 밴드의 시작 dy. **패널 밑단 프레임선이 dy 149** 이고 프롬프트 글자는
/// 그 아래 dy 150.. 이다(실측 80차-D, S9 논리 492x239: 셸 밑단 글자 dy 151..162, 제련 재료창
/// dy 150..156). ⛔ 이보다 위로 내리지 말 것 — dy 141..147 에 **패널 안의 `PLAY TIME` 줄**이
/// 있어서 밴드에 들어오면 그것도 프롬프트 잉크로 읽힌다.
pub const FOOTER_TOP: i32 = 150;
/// 프롬프트 **사이**로 볼 최소 빈 열 수. 실측(80차-D): 프롬프트 사이 간격은 54(게임시작 목록)
/// 와 39(온라인 목록)이고, **프롬프트 안**의 최대 간격은 5(토큰과 콜론 사이)다.
pub const FOOTER_GAP_MIN: i32 = 12;
/// 글자 좌우로 이만큼 키운다. ⛔ 프롬프트 사이 빈 칸을 절반씩 나눠 갖게 하지 말 것 — 그러면
/// 밑단 가운데를 빗나간 엄지가 「초기화」가 된다.
pub const FOOTER_PAD: i32 = 3;

/// ★★★ 밑단 프롬프트 히트 상자가 패널 **아래로** 넘어가는 줄 수 (80차-F).
///
/// 플레이어 제보(2026-09-05): *"게임시작 초기화 및 이어하기 터치영역도 너무 작아서 어디를
/// 터치해야 버튼이 동작하는지 모르겠어"*. 80차-D 의 상자는 `dy 149..159`, **11 논리px** —
/// S9(창 2220x1080 / 논리 492x239)에서 **50 창px ≈ 12 dp** 다. 그 절반은 계산 실수가 아니라
/// **패널이 잘라먹은 것**이다: `#` 글리프의 실측 범위가 dy 155..**161** 인데 패널이 160 에서
/// 끝나므로, 글자의 마지막 두 줄을 누른 엄지를 [`panel_point`] 가 통째로 버리고 있었다.
///
/// 패널 밑단 아래는 게임 배경뿐이고, 우리 컨트롤은 `overlay::hit(&layout.buttons, ..)` 가
/// **이 arm 보다 먼저** 시험하므로(app.rs 터치 체인) 그 밴드를 받아도 아무것도 안 뺏는다.
/// 받는 입구는 [`panel_point_footer`] 하나뿐이고, `menu_tap_decide` 는 그 밴드에서
/// **밑단 프롬프트 둘만** 답하게 한다.
pub const FOOTER_OVERHANG: i32 = 14;
/// 밑단 프롬프트 히트 상자의 **위 끝** (패널 상대).
///
/// 패널 밑단 프레임선이 dy 149 이고 글자는 그 아래다. 여기서 그 위로 조금 더 올라가는 것은
/// 안전하다 — 그 위 dy 141..147 은 패널 안의 `PLAY TIME` 줄이라 **타깃이 아니다**.
/// ⛔ 이보다 더 올리지 말 것: 인벤토리의 페이지 배지 잉크가 dy 131..141 이고, 배지는
/// `menu_tap_decide` 에서 먼저 시험되지만 그 상자는 두 열(폭 29)뿐이라 밑단 가운데는
/// 프롬프트의 것이다 — 밴드를 더 올리면 배지 열 **밖**의 잉크까지 프롬프트 상자가 덮는다.
/// [`FOOTER_BOX_TOP`]..([`PANEL_H`] + [`FOOTER_OVERHANG`]) = **31 논리px ≈ 35 dp**.
pub const FOOTER_BOX_TOP: i32 = 143;
/// 밑단 프롬프트 히트 상자의 아래 끝(경계, 미포함) — 패널 밖으로 [`FOOTER_OVERHANG`] 만큼.
pub const FOOTER_BOX_BOT: i32 = PANEL_H + FOOTER_OVERHANG;
/// ★★ `#` 글리프를 **읽는** 밴드의 아래 끝(경계, 미포함) — 패널 밖으로 넉 줄 (80차-P).
///
/// 글리프의 세로 위치는 상수가 아니다: `wie_probe`(496x240)는 dy **153..159** 에, S9(논리
/// 492x239)는 dy **155..161** 에 그린다 — 게임이 밑단 글자를 패널 원점에서 반올림된 자리에
/// 놓기 때문이다. 80차-D 의 스캔은 `PANEL_H`(160) 에서 멈춰서 S9 프레임에서는 일곱 줄 중
/// **다섯**만 보았고(가로막대 157·159 = [`HASH_BAR_ROWS_MIN`] 과 정확히 같다), 글리프가
/// 한 줄만 더 내려가면 막대 하나를 잃어 `#` 을 못 찾는다 — 그러면 캐릭터를 한 번 로드한
/// 세션(`level > 0`)에서 [`ok_prompt`] 가 밑단 **전체**를 돌려줘 「초기화」 탭이 OK 가 된다.
/// 코드리뷰(2026-09-06)가 잡았다. 네 줄이면 두 실측 위치 모두에서 일곱 줄이 다 들어오고
/// 아래로 2px 더 여유가 있다.
/// ⛔ [`FOOTER_BOX_BOT`](174) 까지 내리지 말 것 — 그 아래는 게임 배경이고, 흰 픽셀 하나가
/// 마스크 검사(빈 행이 아니면 막대/기둥이어야 한다)를 깨서 멀쩡한 `#` 을 잃게 만든다.
pub const FOOTER_SCAN_BOT: i32 = PANEL_H + 4;
/// `#` 글리프의 폭(실측 7px), 그리고 그 안의 두 가로막대 / 세로기둥 행의 최소 개수.
pub const HASH_W: i32 = 7;
pub const HASH_BAR_ROWS_MIN: i32 = 2;
pub const HASH_STEM_ROWS_MIN: i32 = 2;
/// 콜론을 찾을 창: `#` 오른쪽 이만큼 안에 흰 잉크가 있어야 프롬프트로 본다(실측 간격 3).
pub const HASH_COLON_MAX: i32 = 8;

/// 패널 밑단의 `# : …` 프롬프트를 찾아 그 히트 사각형을 돌려준다. 없으면 `None`.
///
/// 플레이어 요청(2026-09-05): *"다음 세션에서 초기화버튼도 터치로 되게 작업 이어서 진행하자"*.
/// 게임시작 슬롯 목록의 밑단은 프롬프트가 **둘**(`OK : 이어하기` 와 `# : 초기화`)이고
/// [`ok_prompt`] 는 밴드 **전체**를 돌려주므로, 그대로 열면 「초기화」를 눌렀을 때 OK(이어하기)가
/// 나간다 — 80차-B 가 그 감지기 하나만 라이브 캐릭터 뒤에 남겨 둔 이유다. 여기서는 반대로
/// **글자를 찾는다**: `#` 글리프 자신을 비트맵으로 확인하고 그 프롬프트의 런만 claim 한다.
///
/// ★★ 토큰 어휘는 게스트 문자열로 **전수 확정**했다(80차-D). 밑단 프롬프트 문자열은
/// `0x5c15c`~`0x5de8c` 의 열셋뿐이고(`@OK:}확인` · `@#:}삭제` · `@:이어하기` · `@#:}초기화` ·
/// `@:메인메뉴` · `@:선택` · `@OK:}제련하기` · `CLR:뒤로` …), 선행 토큰은 **`OK`(ASCII) ·
/// `OK`(크림 스프라이트) · `#` · `CLR`** 넷뿐이다. 그중 `#` 은 언제나 저 작은 흰 글꼴이라
/// 비트맵 하나로 다른 셋과 갈린다 — ⛔ **색으로 가르려 하지 말 것**: 셸의 `CLR` 스프라이트는
/// `OK` 스프라이트와 **같은 크림 계열**이고(실측 m4: `OK` 토큰 폭 22 · `CLR` 33), 동작 이름은
/// 흰색(`이어하기`)과 주황(`초기화`) 둘 다 나온다.
///
/// 비트맵 지문(실측, 논리 492x239 · 패널 상대 dx 126..132 / dy 155..161):
/// ```text
///  155  XX.XX     세로기둥 둘  (mask 0b011_0110)
///  156  XX.XX
///  157  XXXXXXX   가로막대     (mask 0b111_1111)
///  158  XX.XX
///  159  XXXXXXX   가로막대
///  160  XX.XX     ← dy 160,161 은 패널(160) 밖 — 80차-P 부터 [`FOOTER_SCAN_BOT`] 까지 읽는다
///  161  XX.XX
/// ```
/// (`wie_probe` 496x240 에서는 같은 글리프가 dy 153..159 다 — 위치는 해상도에 따라 움직인다.)
/// 밴드 안에서 **빈 행이 아닌 모든 행이 그 두 마스크 중 하나**여야 하고, 양옆 한 칸은 비어
/// 있어야 하며, 오른쪽 [`HASH_COLON_MAX`] 안에 콜론(흰 잉크)이 있어야 한다.
pub fn hash_prompt(px: &[u32], lw: i32, lh: i32) -> Option<Box2> {
    if lw <= 0 || lh <= 0 || px.len() != (lw as usize) * (lh as usize) {
        return None;
    }
    let (ox, oy) = panel_origin(lw, lh)?;
    let is = |dx: i32, dy: i32, want: u32| -> bool {
        let (x, y) = (ox + dx, oy + dy);
        x >= 0 && x < lw && y >= 0 && y < lh && px[(y as usize) * (lw as usize) + x as usize] & 0x00ff_ffff == want
    };
    let white = |dx: i32, dy: i32| is(dx, dy, FOOTER_INK_WHITE);
    // 프롬프트 본문에 올 수 있는 잉크 — 흰 동작 이름과 주황 동작 이름.
    let text = |dx: i32, dy: i32| white(dx, dy) || is(dx, dy, OK_PROMPT_INK);
    // 패널 밖은 「비어 있다」로 본다: 격리 검사가 패널 경계에서 남의 오버레이를 읽으면 안 된다.
    let blank_col = |dx: i32| dx < 0 || dx >= PANEL_W || !(FOOTER_TOP..FOOTER_SCAN_BOT).any(|dy| white(dx, dy));

    let mut hash: Option<i32> = None;
    for dx0 in 0..=(PANEL_W - HASH_W) {
        if !blank_col(dx0 - 1) || !blank_col(dx0 + HASH_W) {
            continue;
        }
        let (mut bar, mut stem, mut ok) = (0, 0, true);
        for dy in FOOTER_TOP..FOOTER_SCAN_BOT {
            let mut mask = 0u32;
            for i in 0..HASH_W {
                if white(dx0 + i, dy) {
                    mask |= 1 << i;
                }
            }
            match mask {
                0 => {}
                0b111_1111 => bar += 1,
                0b011_0110 => stem += 1,
                // 다른 글리프가 이 창에 걸려 있다 — `#` 이 아니다.
                _ => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok || bar < HASH_BAR_ROWS_MIN || stem < HASH_STEM_ROWS_MIN {
            continue;
        }
        // 콜론이 없으면 프롬프트가 아니다.
        if !(1..=HASH_COLON_MAX).any(|d| (FOOTER_TOP..FOOTER_SCAN_BOT).any(|dy| white(dx0 + HASH_W + d, dy))) {
            continue;
        }
        hash = Some(dx0);
        break;
    }
    let dx0 = hash?;

    // 이 프롬프트의 런 오른쪽 끝 — 빈 열이 `FOOTER_GAP_MIN` 이어지면 다음 프롬프트다.
    let mut end = dx0 + HASH_W - 1;
    let mut gap = 0;
    for dx in (dx0 + HASH_W)..PANEL_W {
        if (FOOTER_TOP..FOOTER_SCAN_BOT).any(|dy| text(dx, dy)) {
            end = dx;
            gap = 0;
        } else {
            gap += 1;
            if gap >= FOOTER_GAP_MIN {
                break;
            }
        }
    }
    let x0 = (dx0 - FOOTER_PAD).max(0);
    let x1 = (end + FOOTER_PAD).min(PANEL_W - 1);
    // 세로는 밑단 밴드 전체 — 위로는 `PLAY TIME` 줄까지, 아래로는 패널 **밖**의 글자 두 줄까지
    // (80차-F). 밑단에는 다른 타깃이 없다.
    Some(Box2 {
        dx: x0,
        dy: FOOTER_BOX_TOP,
        w: x1 - x0 + 1,
        h: FOOTER_BOX_BOT - FOOTER_BOX_TOP,
    })
}

/// ★★★ 밑단 토큰(`OK` · `CLR`)은 **단색이 아니다** — 그래서 여기서만 밝기로 본다 (80차-F).
///
/// 셸의 토큰은 [`OK_PROMPT_INK`] 도 [`FOOTER_INK_WHITE`] 도 아닌 **여러 명암의 크림 스프라이트**
/// 이고(실측 `#ad9a73` 을 포함해 여러 값), 밑단 밴드의 배경은 거의 검다(실측 최대 `#211000`,
/// 합 49). 합 380 은 그 사이의 어디에도 없는 값이다.
pub const FOOTER_LIT_SUM: u32 = 380;
/// 토큰·콜론·동작 이름을 가르는 최소 빈 열 수.
///
/// 80차-F 는 4 였다(S9 스크린샷 재샘플링: 토큰→콜론 8 · 콜론→이름 5). ★ 80차-P 가 **2 로
/// 내렸다**: 진짜 프레임버퍼(`wie_probe`)의 타이틀 OPTION 밑단 `CLR : 메인메뉴` 는 `R` 의
/// 안티에일리어스 꼬리 한 픽셀이 dx 78 에 남아 콜론(81..82) 앞 간격이 **2** 다 — 4 를 요구하면
/// 그 `CLR` 을 못 읽는다(80차-N 이 `OK` 쪽에서 본 것과 같은 함정). 간격 하나로 콜론을 가르지
/// 않게 됐으므로(아래 [`FOOTER_COLON_DASH_H`] 의 모양 검사) 이 값은 「글자 사이는 아니다」만
/// 말하면 된다 — 글자 **안**의 열 간격은 최대 1 이다.
pub const FOOTER_COLON_GAP: i32 = 2;
/// 콜론으로 볼 잉크 런의 최대 폭 (실측 2).
pub const FOOTER_COLON_W: i32 = 3;
/// ★ 콜론의 **모양** (80차-P): 그 열들의 잉크가 세로로 정확히 **두 토막**이고 토막 하나가
/// 이 줄 수 이하여야 한다. 실측 — S9 `:` = dy 154..155 + 157..158 (토막 2줄), 프로브 `:` =
/// dy 153 + 156 (토막 1줄). 글자의 세로획(`L` · `1` · 한글 ㅣ)은 토막이 하나라 걸리지 않고,
/// 안티에일리어스 꼬리 픽셀은 토막이 하나이면서 폭 1 이다.
pub const FOOTER_COLON_DASH_H: i32 = 3;
/// `TOKEN :` 접두의 최대 폭 — **이것이 `OK` 와 `CLR` 을 가르는 값이다.**
///
/// 실측: `OK :` = dx 0..25 = **26** (게임시작 슬롯 목록과 온라인 슬롯 목록에서 바이트 동일),
/// `CLR :` = dx 90..126 = **37**. 30 은 그 사이다.
/// ⛔ 색으로 가르려 하지 말 것 — 두 스프라이트는 **같은 크림 계열**이고(80차-D 실측), 동작
/// 이름의 색도 흰색(`이어하기`·`메인메뉴`)과 주황(`초기화`)이 섞여 나온다.
pub const FOOTER_TOKEN_MAX_W: i32 = 30;

/// 밑단에서 **`OK` 가 앞에 붙은 프롬프트 하나**를 적극적으로 찾아 그 히트 사각형을 돌려준다.
///
/// 플레이어 제보(2026-09-05): *"게임시작 초기화 및 이어하기 터치영역도 너무 작아서 어디를
/// 터치해야 버튼이 동작하는지 모르겠어"*. 「초기화」는 80차-D 가 열었지만 **「이어하기」는 한 번도
/// 눌린 적이 없다** — [`ok_prompt`] 는 밴드 **전체**를 돌려주므로 `CLR : 메인메뉴` 를 OK 로
/// 답할 위험 때문에 `stats.level > 0` 뒤에 갇혀 있고, 그 화면은 콜드부팅에서 `level == 0` 이다
/// (80차-D E2E 의 `slot list OWNS the panel … miss` 가 그 기록이다).
///
/// 그래서 여기서는 **토큰을 적극적으로 식별한다**. `hash_prompt` 가 `#` 에 대해 하는 일과 같은
/// 성질이되, 셸의 `OK`/`CLR` 스프라이트는 비트맵으로 못 가른다(안티에일리어스된 스프라이트라
/// 단색이 아니다) — 대신 **`TOKEN :` 접두의 폭**이 가른다. 근거는 두 화면의 실측:
/// ```text
///   게임시작 목록  OK  0..8 10..15   :  24..25   이어하기 31..71     -> 접두 26
///   온라인  목록  OK  0..8 10..15   :  24..25   선택     31..50     -> 접두 26
///   온라인  목록  CLR 90..96 100..101 111..116  :  125..126  메인메뉴 -> 접두 37
/// ```
/// 콜론은 **양옆이 [`FOOTER_COLON_GAP`] 이상 비어 있는 폭 [`FOOTER_COLON_W`] 이하의 런**으로
/// 잡는다(실측 앞 8 · 뒤 5, 글자 사이는 최대 3). 마지막 런은 콜론으로 안 본다 — 콜론 뒤에는
/// 반드시 동작 이름이 있다.
///
/// ⛔ **FAIL CLOSED**: 자격 있는 프롬프트가 **정확히 하나**일 때만 답한다. 못 찾으면 `None`
/// 이고, 그러면 부르는 쪽이 예전의 `stats.level > 0` + [`ok_prompt`] 로 그대로 떨어진다 —
/// 즉 이 함수는 타깃을 **더하기만** 하고 뺏지 않는다.
/// ⛔ `#` 프롬프트의 런은 [`hash_prompt`] 것이므로 제외한다(그 접두도 30 이하다).
pub fn ok_token_prompt(px: &[u32], lw: i32, lh: i32) -> Option<Box2> {
    let toks = footer_token_prompts(px, lw, lh);
    let mut oks = toks.iter().filter(|(t, _)| *t == FooterToken::Ok);
    match (oks.next(), oks.next()) {
        (Some((_, b)), None) => Some(*b),
        _ => None,
    }
}

/// 밑단 프롬프트의 선행 토큰 — [`footer_token_prompts`] 가 폭으로 식별한 것.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FooterToken {
    /// `OK :` — 접두 폭 [`FOOTER_TOKEN_MAX_W`] 이하 (실측 26).
    Ok,
    /// `CLR :` — 접두 폭 [`FOOTER_CLR_TOKEN_MIN_W`]..=[`FOOTER_CLR_TOKEN_MAX_W`] (실측 37, 두 화면).
    Clr,
}

/// `CLR :` 접두 폭의 하한/상한 (80차-P).
///
/// 실측 둘이 **같은 값**이다: 온라인 슬롯 목록 `CLR : 메인메뉴` = dx 90..126 (**37**, S9 기기,
/// 80차-F), 타이틀 환경설정(OPTION) 창 밑단 `CLR : 메인메뉴` = dx 46..82 (**37**, `wie_probe`
/// 496x240, 80차-P). 같은 스프라이트라 폭이 같다. 31..=44 는 그 값의 ±7 이고 `OK :`(26) 과
/// 겹치지 않는다.
/// ⛔ 이 토큰을 식별하는 이유는 **OK 로 오답하지 않기 위해서**가 절반이다: 캐릭터를 한 번
/// 로드한 세션(`level > 0`)의 온라인 목록에서 [`ok_prompt`] 는 밑단 **전체**를 돌려주므로
/// 「메인메뉴」 탭이 OK(= 슬롯 로드)가 됐다 — 코드리뷰(2026-09-06)가 잡은 구멍. 토큰이 하나라도
/// 식별된 밑단에서는 그 낱말들만 답하고 나머지는 무동작이다(`app.rs::footer_prompt_hit`).
pub const FOOTER_CLR_TOKEN_MIN_W: i32 = 31;
pub const FOOTER_CLR_TOKEN_MAX_W: i32 = 44;

/// 밑단의 프롬프트들을 낱말 단위로 갈라 **선행 토큰이 식별된 것들**을 돌려준다 (80차-P).
///
/// [`ok_token_prompt`] 의 본체를 일반화한 것 — 규칙은 그대로다(런 → 프롬프트 묶음 → 콜론 →
/// 접두 폭), 다만 폭이 `CLR :` 대역에 드는 프롬프트도 [`FooterToken::Clr`] 로 돌려준다.
/// 식별이 안 되는 프롬프트는 목록에 **없다**(fail closed). `#` 프롬프트의 런은 [`hash_prompt`]
/// 것이라 제외하고, `#` 이 있으면 나머지 하나는 콜론 없이 `OK` 로 본다(80차-N 의 규칙).
///
/// ⛔ **셸에서만 쓸 것.** 이 규칙이 재어진 것은 셸의 **크림 스프라이트** 토큰이다. 인게임 밑단은
/// 작은 ASCII 글꼴이고, 그 화면들에서는 `PLAY TIME`·`LV1`·STATUS 의 스탯 표(`ATT 58 …`)가 같은
/// 밴드에 들어와서 폭 1~2 의 고립된 런이 「콜론」으로 읽힐 수 있다 — 프로브 프레임(80차-P,
/// STATUS 창)에서 그런 후보가 **둘** 나와 「둘이면 거절」에 걸려 살았을 뿐이다. 인게임은
/// 원래대로 `level > 0` + [`ok_prompt`](주황 동작 이름) 가 받는다.
pub fn footer_token_prompts(px: &[u32], lw: i32, lh: i32) -> Vec<(FooterToken, Box2)> {
    let mut out = Vec::new();
    if lw <= 0 || lh <= 0 || px.len() != (lw as usize) * (lh as usize) {
        return out;
    }
    let Some((ox, oy)) = panel_origin(lw, lh) else { return out };
    let lit = |dx: i32, dy: i32| -> bool {
        let (x, y) = (ox + dx, oy + dy);
        if x < 0 || x >= lw || y < 0 || y >= lh {
            return false;
        }
        let p = px[(y as usize) * (lw as usize) + x as usize];
        u32::from((p >> 16) as u8) + u32::from((p >> 8) as u8) + u32::from(p as u8) >= FOOTER_LIT_SUM
    };
    // 밴드는 **패널 안**만 본다. 아래로 넘어가면 인게임에서는 지도가 밝아 온통 잉크가 된다.
    let col = |dx: i32| (FOOTER_TOP..PANEL_H).any(|dy| lit(dx, dy));

    // 잉크 런(글자 하나 단위)들.
    let mut runs: Vec<(i32, i32)> = Vec::new();
    let mut start: Option<i32> = None;
    for dx in 0..=PANEL_W {
        match (start, dx < PANEL_W && col(dx)) {
            (None, true) => start = Some(dx),
            (Some(a), false) => {
                runs.push((a, dx - 1));
                start = None;
            }
            _ => {}
        }
    }
    if runs.is_empty() {
        return out;
    }

    // 프롬프트 단위로 묶는다 — 프롬프트 **사이**는 `FOOTER_GAP_MIN` 이상 비어 있다.
    let mut groups: Vec<Vec<(i32, i32)>> = vec![vec![runs[0]]];
    for &r in &runs[1..] {
        let prev = *groups.last().and_then(|g| g.last()).expect("groups start non-empty");
        if r.0 - prev.1 - 1 >= FOOTER_GAP_MIN {
            groups.push(vec![r]);
        } else {
            groups.last_mut().expect("groups start non-empty").push(r);
        }
    }

    // 콜론의 모양: 열 `a..=b` 의 잉크 행이 정확히 두 토막, 토막마다 `FOOTER_COLON_DASH_H` 이하.
    let two_dashes = |a: i32, b: i32| -> bool {
        let (mut groups, mut cur, mut short) = (0, 0, true);
        for dy in FOOTER_TOP..PANEL_H {
            if (a..=b).any(|dx| lit(dx, dy)) {
                cur += 1;
                short &= cur <= FOOTER_COLON_DASH_H;
            } else if cur > 0 {
                groups += 1;
                cur = 0;
            }
        }
        if cur > 0 {
            groups += 1;
        }
        short && groups == 2
    };

    let hash = hash_prompt(px, lw, lh);
    // ★★★ 80차-N — **`#` 이 있으면 나머지 하나가 곧 `OK` 다.**
    //
    // 80차-F 의 폭 규칙(`OK :` 26 vs `CLR :` 37)은 콜론을 찾아야 하고, 콜론을 「양옆이 넉넉히
    // 빈 좁은 런」으로 찾는 것은 **안티에일리어스 한 픽셀에 흔들린다**(프로브 프레임에서는
    // `OK` 스프라이트가 dx 0..8·10..17·21 로 나와 콜론 앞 간격이 2 밖에 안 된다 — 기기
    // 스크린샷을 다시 샘플링해 잰 값과 다르다). 그래서 콜론이 필요 없는 길을 하나 더 둔다.
    //
    // 근거는 픽셀이 아니라 **게스트 문자열 전수 목록**(80차-D)이다: `#` 이 들어간 밑단 문자열은
    // `@#:}삭제`(캐릭터 화면)와 `@#:}초기화`(게임시작 목록) 둘뿐이고, 빈 슬롯의 `#:복구` 까지
    // 셋 다 **나머지 프롬프트가 `OK :` 다**(`캐릭터삭제` · `이어하기` · `캐릭터생성`).
    // 캐릭터 화면의 `OK : 캐릭터삭제` 도 OK 를 보내는 것이 **라벨 그대로**라 안전하다 —
    // 80차-D 가 막으려던 것은 `CLR` 라벨에 OK 를 보내는 것이었고, `CLR` 은 `#` 과 같은 밑단에
    // 나오지 않는다.
    if let Some(h) = hash {
        let mut rest = groups.iter().filter(|g| {
            let (g0, g1) = (g[0].0, g.last().expect("a group has a run").1);
            !(g0 <= h.dx + h.w - 1 && h.dx <= g1)
        });
        if let (Some(g), None) = (rest.next(), rest.next()) {
            let (g0, g1) = (g[0].0, g.last().expect("a group has a run").1);
            let x0 = (g0 - FOOTER_PAD).max(0);
            out.push((
                FooterToken::Ok,
                Box2 {
                    dx: x0,
                    dy: FOOTER_BOX_TOP,
                    w: (g1 + FOOTER_PAD).min(PANEL_W - 1) - x0 + 1,
                    h: FOOTER_BOX_BOT - FOOTER_BOX_TOP,
                },
            ));
            return out;
        }
    }
    for g in &groups {
        let (g0, g1) = (g[0].0, g.last().expect("a group has a run").1);
        // `#` 프롬프트의 자리는 그쪽 arm 의 것이다.
        if hash.is_some_and(|h| g0 <= h.dx + h.w - 1 && h.dx <= g1) {
            continue;
        }
        // 콜론: 양옆이 비어 있는 좁은 런이되, **세로로 두 토막**인 것 (80차-P). 마지막 런은
        // 아니다(뒤에 이름이 온다).
        let colon = (1..g.len().saturating_sub(1)).find(|&i| {
            g[i].1 - g[i].0 + 1 <= FOOTER_COLON_W
                && g[i].0 - g[i - 1].1 - 1 >= FOOTER_COLON_GAP
                && g[i + 1].0 - g[i].1 - 1 >= FOOTER_COLON_GAP
                && two_dashes(g[i].0, g[i].1)
        });
        let Some(i) = colon else { continue };
        let prefix = g[i].1 - g0 + 1;
        let token = if prefix <= FOOTER_TOKEN_MAX_W {
            FooterToken::Ok
        } else if (FOOTER_CLR_TOKEN_MIN_W..=FOOTER_CLR_TOKEN_MAX_W).contains(&prefix) {
            FooterToken::Clr
        } else {
            continue;
        };
        out.push((
            token,
            Box2 {
                dx: (g0 - FOOTER_PAD).max(0),
                dy: FOOTER_BOX_TOP,
                w: (g1 + FOOTER_PAD).min(PANEL_W - 1) - (g0 - FOOTER_PAD).max(0) + 1,
                h: FOOTER_BOX_BOT - FOOTER_BOX_TOP,
            },
        ));
    }
    out
}

/// 메시지 박스 밑단의 **가운데 `OK` 버튼**: 밝은 회색 띠(189,178,173) 위에 흰 `OK` 글자.
/// 실측(66차-B): 제련 결과창("제련에 성공하였습니다. 콜드 +1")과 상점의 대화창이 같은 패턴이고,
/// 띠는 dy 99..113 / 폭 150 안쪽, 흰 글자는 가운데 15px 남짓이다.
pub const OK_BTN_BAND_INK: u32 = 0x00bd_b2ad;
pub const OK_BTN_TEXT_INK: u32 = 0x00ff_ffff;
/// 띠로 볼 최소 폭·높이 범위.
pub const OK_BTN_BAND_MIN_W: i32 = 100;
pub const OK_BTN_H_MIN: i32 = 8;
pub const OK_BTN_H_MAX: i32 = 20;
/// 흰 글자의 최소 픽셀 수, 그리고 **가로 폭 상한**.
///
/// ⛔ 그 폭 상한이 이 감지기의 안전장치다: `예/아니요` 확인창의 본문도 같은 회색이고 흰 글자를
/// 담고 있는데, 그 글자는 두 버튼에 걸쳐 **115px** 로 퍼져 있다. `OK` 두 글자는 15px 이고
/// 가운데 있으므로, 폭과 중심으로 둘을 가른다.
pub const OK_BTN_TEXT_MIN: i32 = 10;
pub const OK_BTN_TEXT_MAX_W: i32 = 40;
pub const OK_BTN_TEXT_OFF_CENTRE: i32 = 20;

/// 메시지 박스의 가운데 `OK` 버튼 띠. 없으면 `None`.
///
/// 플레이어 요청(2026-08-23): *"제련이 성공하였습니다 창에서 OK버튼 눌렀을때 동작하게"*.
/// 게임이 그린 버튼이므로 보내는 것은 OK 키 하나뿐이다.
pub fn ok_button(px: &[u32], lw: i32, lh: i32) -> Option<Box2> {
    if lw <= 0 || lh <= 0 || px.len() != (lw as usize) * (lh as usize) {
        return None;
    }
    let (ox, oy) = panel_origin(lw, lh)?;
    let ink = |dx: i32, dy: i32| -> Option<u32> {
        let (x, y) = (ox + dx, oy + dy);
        (x >= 0 && x < lw && y >= 0 && y < lh).then(|| px[(y as usize) * (lw as usize) + x as usize] & 0x00ff_ffff)
    };
    // 회색 띠의 행들 (폭이 넉넉한 것만) -> 밴드로 묶는다.
    let mut bands: Vec<(i32, i32, i32, i32)> = Vec::new(); // (dy0, dy1, x0, x1)
    for dy in 0..PANEL_H {
        let mut first = None;
        let mut last = 0;
        let mut n = 0;
        for dx in 0..PANEL_W {
            if ink(dx, dy) == Some(OK_BTN_BAND_INK) {
                first = first.or(Some(dx));
                last = dx;
                n += 1;
            }
        }
        let Some(a) = first else { continue };
        if n < OK_BTN_BAND_MIN_W {
            continue;
        }
        match bands.last_mut() {
            Some(b) if b.1 == dy - 1 => {
                b.1 = dy;
                b.2 = b.2.min(a);
                b.3 = b.3.max(last);
            }
            _ => bands.push((dy, dy, a, last)),
        }
    }
    for &(dy0, dy1, x0, x1) in &bands {
        let h = dy1 - dy0 + 1;
        if !(OK_BTN_H_MIN..=OK_BTN_H_MAX).contains(&h) {
            continue;
        }
        // 그 띠 안의 흰 글자: 개수와 **폭**과 중심.
        let mut n = 0;
        let (mut tx0, mut tx1) = (PANEL_W, -1);
        for dy in dy0..=dy1 {
            for dx in x0..=x1 {
                if ink(dx, dy) == Some(OK_BTN_TEXT_INK) {
                    n += 1;
                    tx0 = tx0.min(dx);
                    tx1 = tx1.max(dx);
                }
            }
        }
        if n < OK_BTN_TEXT_MIN || tx1 < tx0 {
            continue;
        }
        let w = tx1 - tx0 + 1;
        let centre = (tx0 + tx1) / 2;
        if w > OK_BTN_TEXT_MAX_W || (centre - PANEL_W / 2).abs() > OK_BTN_TEXT_OFF_CENTRE {
            continue;
        }
        return Some(Box2 { dx: x0, dy: dy0, w: x1 - x0 + 1, h });
    }
    None
}

/// 배지 글리프의 잉크 (RGB, 알파 없음). 헤더 제목 글자는 (214,178,**156**)로 8만
/// 다르지만, x 열 제한이 그 모호성을 없앤다.
pub const BADGE_INK: u32 = 0x00d6_b294;
/// 좌/우 배지 열의 x 와 폭.
pub const BADGE_L_X: i32 = 15;
pub const BADGE_R_X: i32 = 151;
pub const BADGE_W: i32 = 11;
/// 배지 히트 영역을 잉크보다 이만큼 키운다 (D-패드 화살표와 같은 정책:
/// "표시되는 영역보다 조금 더 큰 범위를 눌러도 동작하게", 2026-08-17).
pub const BADGE_PAD: i32 = 4;
/// 세로로는 이만큼 키운다 — 가로보다 넉넉하다 (80차-F).
///
/// 잉크 밴드가 11px 뿐이라 예전 `2*4+11 = 19` 논리px 는 S9(논리 492x239, 창 2220x1080)에서
/// **86 창px ≈ 21 dp** 였다. 27 논리px = 122 창px ≈ 30 dp.
/// ⛔ 이보다 키우지 말 것 — 탭 배지의 잉크가 dy 2..12 라 상자 밑단이 dy 20 이 되는데,
/// **아이템 격자의 첫 행이 dy 22~23** 이다(실측 79차-E/80차-E). 배지는
/// `menu_tap_decide` 에서 격자보다 **먼저** 시험하므로, 겹치는 순간 첫 행 칸이 통째로
/// 안 눌린다.
pub const BADGE_PAD_Y: i32 = 8;
/// 배지 플레이트 **바깥**에 게임이 스스로 그리는 `<<` / `>>` 화살표까지의 폭.
///
/// 플레이어 요청(2026-09-05): *"1,3,7,9 문구박스에서 >> << 이 버튼까지 포함해서 터치영역을
/// 차지하도록"*. 실측(80차-F, S9 논리 492x239, INVENTORY):
/// ```text
///   <<        «1»            «3»        >>
///   dx 5..11  dx 15..25      dx 151..161  dx 163..171     (헤더 dy 4..10 / 2..12)
///   dx 5..11  dx 15..25      dx 151..161  dx 163..171     (밑단 dy 133..139 / 131..141)
/// ```
/// 즉 화살표는 플레이트에서 **바깥으로 10px** 이고, 헤더와 밑단이 같다. 이 상수는 그 바깥쪽
/// 확장폭이고 [`BADGE_PAD`] 가 그 위에 또 얹힌다 → 좌 상자 dx 1..29 · 우 상자 dx 147..175.
/// ⛔ "패널 가장자리까지"로 넓히지 말 것 — 스킬/퀘스트의 페이지 배지는 dy 145..155 라
/// 상자가 밑단 프롬프트 밴드에 걸치는데, 배지가 먼저 시험되므로 폭이 패널 전체가 되면
/// `OK : 선택` 프롬프트가 통째로 사라진다.
pub const BADGE_ARROW_W: i32 = 10;
/// 한 배지로 인정할 잉크 픽셀 수의 하한. 실측은 열당 84~93 px 이다.
pub const BADGE_MIN_INK: i32 = 20;
/// 배지 밴드의 높이 허용 범위 (실측 11).
pub const BADGE_BAND_MIN_H: i32 = 8;
pub const BADGE_BAND_MAX_H: i32 = 14;
/// 밴드 분류: 이보다 위에서 끝나면 탭 배지, 이보다 아래에서 시작하면 페이지 배지.
/// 그 사이는 아무것도 아니다 (모르는 것에 타깃을 만들지 않는다).
pub const BADGE_TAB_MAX_DY: i32 = 20;
pub const BADGE_PAGE_MIN_DY: i32 = 100;

/// 게임이 그린 네 배지. 키는 게임이 인쇄한 글자 그대로다 (`1` `3` `7` `9`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Badge {
    /// 좌상단 «1» — 이전 탭.
    Tab1,
    /// 우상단 «3» — 다음 탭.
    Tab3,
    /// 좌하단 «7» — 이전 페이지(가방).
    Page7,
    /// 우하단 «9» — 다음 페이지(가방).
    Page9,
}

/// 스캔으로 실제로 찾은 배지 하나 — 패널 상대 히트 사각형까지 (이미 PAD 만큼 커진 값).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BadgeHit {
    pub badge: Badge,
    pub dx: i32,
    pub dy: i32,
    pub w: i32,
    pub h: i32,
}

impl BadgeHit {
    pub fn contains(&self, dx: i32, dy: i32) -> bool {
        dx >= self.dx && dx < self.dx + self.w && dy >= self.dy && dy < self.dy + self.h
    }
}

/// 패널의 좌상단 (논리 게스트px). 프레임버퍼가 패널보다 작으면 `None` —
/// 그런 화면에서는 게임도 패널을 그렇게 그리지 않으므로 추측하지 않는다.
pub fn panel_origin(lw: i32, lh: i32) -> Option<(i32, i32)> {
    (lw >= PANEL_W && lh >= PANEL_H).then(|| ((lw - PANEL_W) / 2, (lh - PANEL_H) / 2))
}

/// 논리 좌표 -> 패널 상대 좌표. 패널 밖이면 `None`.
pub fn panel_point(lw: i32, lh: i32, x: i32, y: i32) -> Option<(i32, i32)> {
    let (px, py) = panel_origin(lw, lh)?;
    let (dx, dy) = (x - px, y - py);
    (dx >= 0 && dx < PANEL_W && dy >= 0 && dy < PANEL_H).then_some((dx, dy))
}

/// [`panel_point`] 과 같되 **아래로 [`FOOTER_OVERHANG`] 줄까지** 더 받는다 (80차-F).
///
/// 돌려주는 `dy` 는 `PANEL_H` 이상일 수 있다 — 그 값은 「패널 밑단 바깥」이라는 뜻이고,
/// 부르는 쪽은 거기서 **밑단 프롬프트 둘만** 답하게 해야 한다. 패널 안을 해석하는 감지기들은
/// 전부 자기 상자로 `contains` 를 하므로 자연히 안 걸리지만, 워커로 좌표를 그대로 넘기는
/// 마지막 arm 은 그렇지 않다 — `app::menu_tap_decide` 가 그 게이트를 갖고 있다.
pub fn panel_point_footer(lw: i32, lh: i32, x: i32, y: i32) -> Option<(i32, i32)> {
    let (px, py) = panel_origin(lw, lh)?;
    let (dx, dy) = (x - px, y - py);
    (dx >= 0 && dx < PANEL_W && dy >= 0 && dy < FOOTER_BOX_BOT).then_some((dx, dy))
}

/// 논리 프레임버퍼(0xAARRGGBB, 길이 `lw*lh`)의 배지 두 열을 훑어 게임이 실제로
/// 그린 배지만 돌려준다. 프레임이 없거나 크기가 안 맞으면 빈 결과 — 즉 타깃 0개.
///
/// 이것이 1단계의 전부다: 게스트 RE 0, 메모리 쓰기 0, 커서 0. 63차 컷신 `# SKIP`
/// 과 같은 패턴이고, 여기서는 dy 를 가정할 수 없으니 픽셀이 답한다.
pub fn badges(px: &[u32], lw: i32, lh: i32) -> Vec<BadgeHit> {
    let mut out = Vec::new();
    if lw <= 0 || lh <= 0 || px.len() != (lw as usize) * (lh as usize) {
        return out;
    }
    let Some((ox, oy)) = panel_origin(lw, lh) else { return out };
    for (col_x, tab, page) in [(BADGE_L_X, Badge::Tab1, Badge::Page7), (BADGE_R_X, Badge::Tab3, Badge::Page9)] {
        // 이 열의 잉크 밴드들 (연속한 dy 구간).
        let mut band: Option<(i32, i32, i32)> = None; // (dy0, dy1, 잉크 px)
        for dy in 0..PANEL_H {
            let y = oy + dy;
            let mut ink = 0;
            for i in 0..BADGE_W {
                let x = ox + col_x + i;
                if x < 0 || x >= lw || y < 0 || y >= lh {
                    continue;
                }
                if px[(y as usize) * (lw as usize) + x as usize] & 0x00ff_ffff == BADGE_INK {
                    ink += 1;
                }
            }
            band = match (band, ink > 0) {
                (Some((a, _, n)), true) => Some((a, dy, n + ink)),
                (None, true) => Some((dy, dy, ink)),
                (Some(b), false) => {
                    push_band(&mut out, b, col_x, tab, page);
                    None
                }
                (None, false) => None,
            };
        }
        if let Some(b) = band {
            push_band(&mut out, b, col_x, tab, page);
        }
    }
    out
}

/// 한 잉크 밴드를 배지로 인정하고 히트 사각형을 만든다 — 인정 못 하면 버린다.
fn push_band(out: &mut Vec<BadgeHit>, (dy0, dy1, ink): (i32, i32, i32), col_x: i32, tab: Badge, page: Badge) {
    let h = dy1 - dy0 + 1;
    if ink < BADGE_MIN_INK || h < BADGE_BAND_MIN_H || h > BADGE_BAND_MAX_H {
        return;
    }
    let badge = if dy1 <= BADGE_TAB_MAX_DY {
        tab
    } else if dy0 >= BADGE_PAGE_MIN_DY {
        page
    } else {
        return;
    };
    // PAD 만큼 키우고, **바깥쪽**으로는 게임이 그린 `<<` / `>>` 화살표까지 더 먹고,
    // 그다음 패널 안으로 클립 — 타깃이 패널 밖으로 새면 게임 화면의 다른 것을 먹는다.
    // 어느 쪽이 바깥인지는 열이 정한다: 좌열의 화살표는 왼쪽, 우열의 것은 오른쪽이다.
    let left = col_x * 2 < PANEL_W;
    let x0 = col_x - BADGE_PAD - if left { BADGE_ARROW_W } else { 0 };
    let x1 = col_x + BADGE_W - 1 + BADGE_PAD + if left { 0 } else { BADGE_ARROW_W };
    let dx = x0.max(0);
    let dy = (dy0 - BADGE_PAD_Y).max(0);
    let w = (x1 + 1).min(PANEL_W) - dx;
    let hh = (dy1 + 1 + BADGE_PAD_Y).min(PANEL_H) - dy;
    out.push(BadgeHit { badge, dx, dy, w, h: hh });
}

/// 스캔 결과에서 이 점이 누른 배지. 먼저 찾은 것이 이긴다 (겹치지 않는다).
pub fn badge_at(hits: &[BadgeHit], dx: i32, dy: i32) -> Option<Badge> {
    hits.iter().find(|h| h.contains(dx, dy)).map(|h| h.badge)
}


// ── 동작 팝업(드랍박스) ──────────────────────────────────────────────────────
//
// 79차-C. 66차가 남겨 둔 마지막 인벤 항목이고, 막고 있던 것은 **팝업이 화면 어디에
// 그려지는지 알 수 없다**는 것이었다 — 조사 문서(§7 「DRAW OVER THE POPUP」)가 그렇게
// 적었고, 그건 맞았다. 그리는 함수는 x/y 를 **인자로** 받고 그 값은 호출자의 스택
// 프레임 밖으로 나오지 않는다.
//
// ── 그래서 픽셀에서 찾는다 (배지와 같은 수법, 신호는 훨씬 세다) ──────────────
// 정적 RE 로 페인터를 특정했다 — LGT `0x40570(x, y, sel)`, 호출자 셋
// (`0x427da` · `0x435e8` · `0x44588`), 그리고 그 안에서 기하가 **전부 상수**다:
//
// ```text
//   0x405a0  n    = (i8)*(CHAR+0x9c85)          ; 행수 (호스트도 읽는 그 바이트)
//   0x405a6  r5   = 17 * n
//   0x405ac  r5  += 8                            ; 상자 높이 = 17n + 8
//   0x405ce  fill (x, y, 0x4a=74, 17n+8, 0xbdb3ac)
//   0x405e8  rect (x, y, 0x4a=74, 17n+8, 0x000000)   ; 검은 외곽선
//   0x40602  rowY = (y + 4) + 17*i               ; [sp+0x20] = y+4
//   0x4061c  fill (x + 6, rowY, 0x3e=62, 17, 0x7a5b54)
//   0x40640  sel 이면 (x + 6, rowY + 1, 62, 15, 0x4a3026) 로 덧칠
// ```
//
// ⚠ 호출자 둘이 `n > 6` 일 때 y 를 ±14/25 만큼 옮긴다(`0x427be`, `0x4456a`). 그래서
// **y 를 모델링하면 안 되고**, 아래처럼 찾아야 한다.
//
// ── 실측 (wie_probe, LGT 6a352944, 496x240, 인벤 0번 칸 → OK, n=2) ───────────
// 프레임 diff bbox = x 192..265 · y 62..103 → **폭 74**, **높이 42 = 17·2+8**. 정적
// 모델과 픽셀이 정확히 일치한다. 그 안의 색은(호스트 u32 = 0x00RRGGBB):
//   상자 몸통 `0xbdb2ad` (= `CONFIRM2_BG_INK`) · 행 `0x7b5952` · 선택행 `0x4a3121`
// ★ 리터럴(`0xbdb3ac`/`0x7a5b54`/`0x4a3026`)과 다르다 — RGB565 왕복 때문이다. 그래서
//   **측정값을 쓴다** (저장소의 다른 잉크 상수도 전부 그렇다).
//
// ── 감지: 정확히 73px 인 검은 가로 런 두 개 ─────────────────────────────────
// 검은 `drawRect` 는 폭 74 를 받아 **73px** 을 칠한다(우측 열과 최하단 행은 몸통색으로
// 남는다 — x=265 열이 `bdb2ad x42`, y=103 행이 `bdb2ad x74` 로 실측됐다). 그 두 런은
// 같은 x 에서 시작하고 `17n + 6` 만큼 떨어져 있다.
// **팝업이 없는 프레임에서는 정확히-73 런이 0개였고, 있는 프레임에서는 정확히 2개였다**
// (8400·8800 = 0, 9000·9200·9350 = [(192,62),(192,102)]). 행수를 이미 알고 있으므로
// 간격까지 맞춰 보는 셈이고, 거기에 몸통색 두 점(오른쪽 열·맨 아랫줄)까지 확인한다.
//
// ⚠ 실패 방향은 안전한 쪽이다: 팝업 주변이 검게 물들어 런이 이어져 버리면 상자를
// **못 찾고**, 그러면 탭이 예전처럼 아무 일도 하지 않는다 (잘못된 행을 누르지 않는다).

/// 팝업 상자의 폭 (논리 게스트px) — 페인터의 `movs r3,#0x4a`.
pub const POPUP_W: i32 = 74;
/// 검은 외곽선이 실제로 칠하는 가로 길이. ⛔ `POPUP_W` 가 아니다 (위 참조).
pub const POPUP_RULE_W: i32 = 73;
/// 한 행의 높이이자 세로 간격 — 페인터의 `movs r7,#0x11`.
pub const POPUP_ROW_H: i32 = 17;
/// 행 목록이 아닌 위아래 여백의 합 — 페인터의 `adds r5,#8`.
pub const POPUP_CHROME_H: i32 = 8;
/// 첫 행이 시작하는 상자 안쪽 y — 페인터의 `adds r3,r6,#4`.
pub const POPUP_ROW_Y0: i32 = 4;
/// 행이 시작하는 상자 안쪽 x 와 그 폭 — `adds r3,r4,#6` 과 `movs r3,#0x3e`.
pub const POPUP_ROW_X0: i32 = 6;
pub const POPUP_ROW_W: i32 = 62;
/// 라벨 버퍼가 8개뿐이므로 행도 그 이상일 수 없다 (`0x3fa46` 의 할당 루프).
pub const POPUP_MAX_ROWS: i32 = 8;

/// 상자 외곽선 색 — 검정. 페인터가 색 인자로 상수 0 을 넘긴다 (`movs r3,#0` @`0x405d8`).
pub const POPUP_RULE_INK: u32 = 0x0000_0000;
/// 상자 몸통 색 (실측 — 리터럴 `0xbdb3ac` 의 RGB565 왕복). 게임의 확인창 배경과 같은
/// 값이라 이것만으로는 팝업을 못 찾지만, 외곽선과 **함께** 보면 정밀도가 오른다.
pub const POPUP_BODY_INK: u32 = 0x00bd_b2ad;

/// 화면에 열려 있는 동작 팝업의 좌상단 (논리 게스트px), 행수 `rows` 를 알고 있을 때.
///
/// `rows` 는 게스트 메모리(`CHAR+0x9c85`)에서 오고 워커가 UI 로 실어 준다. 그것을
/// **간격 검사에 쓰기 때문에** 이 함수는 "비슷한 상자"가 아니라 "이 팝업"을 찾는다.
pub fn popup_box(px: &[u32], lw: i32, lh: i32, rows: i32) -> Option<(i32, i32)> {
    if !(1..=POPUP_MAX_ROWS).contains(&rows) || lw <= POPUP_W || lh <= 0 {
        return None;
    }
    let h = POPUP_ROW_H * rows + POPUP_CHROME_H;
    let at = |x: i32, y: i32| -> Option<u32> {
        (x >= 0 && x < lw && y >= 0 && y < lh).then(|| px[(y * lw + x) as usize] & 0x00ff_ffff)
    };
    // The black `drawRect` stops one short on both axes, so the box's RIGHT column and
    // its BOTTOM row stay body-coloured (measured: x=265 was `bdb2ad` for all 42 rows,
    // y=103 for all 74 columns). Checking those two costs nothing and means a stray
    // pair of black rules cannot be mistaken for a popup.
    let body_ok = |x: i32, y: i32| at(x + POPUP_W - 1, y) == Some(POPUP_BODY_INK) && at(x, y + h - 1) == Some(POPUP_BODY_INK);
    let rules = popup_rules(px, lw, lh);
    rules
        .iter()
        .find(|(x, y)| rules.iter().any(|(x2, y2)| x2 == x && *y2 == y + h - 2) && body_ok(*x, *y))
        .copied()
}

/// 정확히 [`POPUP_RULE_W`] 픽셀인 검은 가로 런의 시작점 전부.
fn popup_rules(px: &[u32], lw: i32, lh: i32) -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    for y in 0..lh {
        let mut x = 0;
        while x < lw {
            if px[(y * lw + x) as usize] & 0x00ff_ffff == POPUP_RULE_INK {
                let x0 = x;
                while x < lw && px[(y * lw + x) as usize] & 0x00ff_ffff == POPUP_RULE_INK {
                    x += 1;
                }
                if x - x0 == POPUP_RULE_W {
                    out.push((x0, y));
                }
            } else {
                x += 1;
            }
        }
    }
    out
}

/// 팝업 상자 `(bx, by)` 안의 점이 누른 행. 상자 밖이거나 행이 아닌 여백이면 `None`.
///
/// ⛔ 클램프가 아니라 **거절**이다. 팝업 커서(`CHAR+0x9c86`)는 게임의 키 arm 밖에서는
/// 범위 검사가 없고, 이 값은 그대로 그 바이트로 간다 (§7-1 과 같은 위험).
pub fn popup_row_at(bx: i32, by: i32, rows: i32, x: i32, y: i32) -> Option<u8> {
    if !(1..=POPUP_MAX_ROWS).contains(&rows) {
        return None;
    }
    let (dx, dy) = (x - bx, y - by);
    if dx < POPUP_ROW_X0 || dx >= POPUP_ROW_X0 + POPUP_ROW_W {
        return None;
    }
    let r = dy - POPUP_ROW_Y0;
    if r < 0 || r >= POPUP_ROW_H * rows {
        return None;
    }
    u8::try_from(r / POPUP_ROW_H).ok()
}

/// 그 점이 팝업 상자 안이기만 한가 — 행이 아니어도. 팝업이 떠 있는 동안 상자 위의 탭이
/// **밑의 격자로 새지 않게** 하는 데 쓴다 (66차의 `POPUP_OPEN_OFF` 거절이 하던 일을
/// 상자 단위로 좁힌 것이다).
pub fn popup_contains(bx: i32, by: i32, rows: i32, x: i32, y: i32) -> bool {
    let h = POPUP_ROW_H * rows + POPUP_CHROME_H;
    (1..=POPUP_MAX_ROWS).contains(&rows) && x >= bx && x < bx + POPUP_W && y >= by && y < by + h
}


// ── 상태보기(STATUS)의 스탯 격자 ────────────────────────────────────────────
//
// 79차-C. 66차-B 가 *"상태보기의 스탯 목록은 올릴 포인트가 있을 때만 존재한다"* 며 남긴
// 항목이고, 그래서 프로브로 **포인트를 만들어서** 쟀다 (`--poke CHAR+0x18a = 10`).
//
// ── 커서를 어떻게 찾았나 (프로브 2런 CHAR 전체 diff) ────────────────────────
// 같은 입력으로 상태보기까지 몰고, 한 런만 `DOWN` 을 하나 더 넣고 `CHAR..+0xa500` 을 통째로
// 떴다. **다른 바이트가 정확히 하나** — `CHAR+0x9c70`, 0 → **2**.
//
// 그 "2" 가 이 화면의 모양을 알려 준다: **2열 격자**이고 `커서 = 행*2 + 열` 이라 한 줄
// 내려가면 +2 다. 커서를 0·1·2·3 으로 **직접 포크**해서(키 0회) 확인했다 —
//
// ```text
//   커서 0 → 패널 dx 80  dy 69      커서 1 → dx 141 dy 69
//   커서 2 → dx 80  dy 83           커서 3 → dx 141 dy 83      전부 31 x 12
//   커서 4..15 → 게임이 마커를 아예 안 그린다  ⇒ 칸은 넷뿐이다
// ```
//
// ★ **해상도 무관**: 496x240 과 560x228 두 논리 크기에서 위 패널 상대 좌표가 **같았다**
//   (66차의 패널 176x160 중앙정렬 성질 그대로).
// ⚠ 커서 4 이상이 아무것도 안 그린다는 것은 `TOUCH_MENU_FEASIBILITY.md §7-1` 의 그 위험을
//   이 화면에서 다시 확인한 것이다 — 범위 밖 커서를 쓰면 게임이 모르는 상태가 된다.
//   `Grid::hit` 은 구조적으로 0..3 밖을 못 돌려준다.

/// 상태보기의 스탯 격자 — 2열 x 2행, 커서 `CHAR+0x9c70`.
pub const STATUS: Grid = Grid {
    x0: 80,
    y0: 69,
    w: 31,
    h: 12,
    pitch_x: 61,
    pitch_y: 14,
    cols: 2,
    rows: 2,
    pad_x: 4,
    pad_y: 2,
};

/// 커서가 앉은 칸에 게임이 그리는 시안색 테두리. 이 화면에서 이 색은 그것뿐이다(실측).
pub const STATUS_MARK_INK: u32 = 0x005a_ebff;

/// 지금 시안 테두리가 앉아 있는 칸 — 없으면 `None`.
///
/// ★★ 이것이 **교차검증**이다. 워커가 라이브 커서(`CHAR+0x9c70`)와 이 값을 대조해서 같을
/// 때만 쓴다 — `items::item_grid_action` 이 노란 칸 테두리로 하는 것과 같은 규약이다.
/// 올릴 포인트가 없어 목록이 아예 없는 화면에서는 테두리가 없으므로 `None` 이 나오고,
/// 그러면 탭은 아무 일도 하지 않는다 (fail closed).
pub fn status_marker(px: &[u32], lw: i32, lh: i32) -> Option<u8> {
    let (ox, oy) = panel_origin(lw, lh)?;
    let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for dy in 0..PANEL_H {
        for dx in 0..PANEL_W {
            let (x, y) = (ox + dx, oy + dy);
            if x < 0 || x >= lw || y < 0 || y >= lh {
                continue;
            }
            if px[(y * lw + x) as usize] & 0x00ff_ffff == STATUS_MARK_INK {
                x0 = x0.min(dx);
                y0 = y0.min(dy);
                x1 = x1.max(dx);
                y1 = y1.max(dy);
            }
        }
    }
    // 시안이 한 픽셀도 없으면 여기서 끝 — ⛔ 이 검사가 없으면 아래 뺄셈이 `MIN - MAX` 로
    // 넘친다 (셀프테스트 `no_marker_means_no_answer` 가 잡았다. 디버그 빌드는 패닉이고
    // 릴리스는 조용히 감기는데, 감긴 값이 우연히 31/12 가 되면 없는 마커를 있다고 한다).
    if x0 == i32::MAX {
        return None;
    }
    // 정확히 한 칸 크기여야 한다 — 다른 화면의 시안 픽셀이 우연히 모여도 이 검사를 못 넘는다.
    if x1 - x0 + 1 != STATUS.w || y1 - y0 + 1 != STATUS.h {
        return None;
    }
    // ...그리고 네 칸 중 하나의 좌상단과 **정확히** 일치해야 한다 (근사가 아니다).
    (0..STATUS.cells()).find(|i| {
        let (col, row) = (i % STATUS.cols, i / STATUS.cols);
        x0 == STATUS.x0 + STATUS.pitch_x * col && y0 == STATUS.y0 + STATUS.pitch_y * row
    })
    .and_then(|i| u8::try_from(i).ok())
}


// ── 장비(EQUIPMENT) 화면의 3x3 격자 ─────────────────────────────────────────
//
// 79차-D. 66차-B 가 *"커서 3바이트 + 빈 슬롯 건너뜀"* 이라며 남긴 항목이고, 이번에는
// **실기기**에서 잤다 (프로브의 맨 캐릭터로는 「건너뛸 것이 없었다」와 구분이 안 된다).
//
// ── 커서는 정말 세 바이트다 ────────────────────────────────────────────────
// `CHAR+0x9c77` 인덱스 · `+0x9c78` 열 · `+0x9c79` 행. 장비 아홉 칸을 다 채운 캐릭터로
// 게임 자신의 방향키를 눌러 가며 `focus probe` 를 읽었다:
//
// ```text
//   RIGHT x10 : idx 0 1 2 3 4 5 6 7 8 → 0        col 0 1 2 0 1 2 0 1 2
//                                                 row 0 0 0 1 1 1 2 2 2
//   DOWN  x5  : idx 0 3 6 → 6 (더 안 감)
// ```
// ⇒ **`idx = row*3 + col`, 아홉 칸, RIGHT 는 감기고 DOWN 은 마지막 행에서 멈춘다.**
//
// ── 기하 (실기기 스크린샷에서 역산, 2340x1080 → 논리 496x228) ───────────────
// 스케일은 **기지값으로 검증**했다: 같은 방식으로 잰 메뉴 0행의 선택 잉크가
// `dx 24.20..151.80 / dy 24.06..38.83` 으로 나왔고, 알려진 값은 `dx 24..151 / dy 24..38` 이다.
// 그 스케일로 노란 칸 테두리를 재면 —
//
// ```text
//   커서 0 → dx  5.12.. 20.81   dy 21.10.. 36.93
//   커서 4 → dx 23.14.. 38.82   dy 39.04.. 54.88
//   커서 8 → dx 41.16.. 56.84   dy 57.20.. 72.82
// ```
// 즉 `x0 5 · y0 21 · 16x16 · 간격 18` 에 세 점이 정확히 앉는다.
//
// ★★★ **80차-I: 「빈 슬롯 건너뜀」 경고는 반증됐다 — 이제 세 바이트를 직접 쓴다.**
//
// 79차-D 는 아홉 칸을 다 착용한 캐릭터로만 걸어 봤기 때문에 그 경고를 죽일 수 없었고, 그래서
// **게임 자신의 RIGHT 를 폴마다 한 번씩** 눌러 옮겼다(`MenuTapAction::Walk`). 플레이어가 본
// 것이 그 대가다 (2026-09-05): *"장비창에서 신발을 터치하면 포커스가 갑자기 다른칸들을
// 쭈르르르륵 순식간에 돈다음에 신발로 포커스이동"*.
//
// 반증 실측 (vc258, S9, **슬롯 1·2·5·8 이 비어 있는 LV3 캐릭터**, 게임 자신의 RIGHT 9회):
// ```text
//   idx  0 1 2 3 4 5 6 7 8 → 0
//   col  0 1 2 0 1 2 0 1 2 → 0
//   row  0 0 0 1 1 1 2 2 2 → 0
// ```
// **빈 칸을 하나도 건너뛰지 않는다.** 당연한 결과이기도 하다 — 이 격자의 아홉 칸은 아이템
// 목록이 아니라 **부위**(투구·갑옷·…)이고, 빈 칸을 포커스할 수 없으면 애초에 그 부위에
// 장비를 낄 수가 없다.
// ⇒ `MenuTapAction::FocusEquip` 이 idx/col/row 세 바이트를 한 번에 쓴다. 세 값 다 게임
// 자신의 방향키가 만드는 범위(0..8 / 0..2 / 0..2) 안이라 범위 밖 상태를 만들 수 없고,
// 노란 테두리 교차검증(`equip_marker`)은 그대로다.

/// 장비 화면의 착용 슬롯 격자 — 3열 x 3행, 커서 `CHAR+0x9c77`(인덱스).
pub const EQUIP: Grid = Grid {
    x0: 5,
    y0: 21,
    w: 16,
    h: 16,
    pitch_x: 18,
    pitch_y: 18,
    cols: 3,
    rows: 3,
    pad_x: 1,
    pad_y: 1,
};

/// 장비 화면 **오른쪽의 후보 목록** 격자 — 「장비장착」을 고르면 커서가 여기로 넘어간다.
///
/// ★★★ 80차-I. 플레이어: *"포커스 된상태에서 장비장착하면 우측에 현재 장비들이뜨는데
/// 거기서는 또 터치가안돼"*. 기기 실측(vc258, S9)으로 두 가지가 확정됐다:
/// ① 기하는 [`EQUIP`] 과 **x 만 다르다** — 노란 테두리 실측 `(col0,row0)` dx 117..132 /
///    dy 21..36, `(col0,row2)` dy 57..72 ⇒ `x0 117 · y0 21 · 16x16 · 간격 18`;
/// ② 커서는 **같은 세 바이트를 나눠 쓴다**. 「장비장착」을 고른 직후 `equip idx/col/row` 가
///    `0/0/0` 이 되고(게스트 `0x483a0` 이 셋을 0 으로 민다), 그 뒤 RIGHT x3 · DOWN x1 이
///    `col 0→1→2→0 / row 0→0→0→1→2` 로 움직이는 동안 **`idx` 는 0 에 머문다**.
/// ⇒ 후보 목록의 커서는 **`col`(`+0x9c78`)과 `row`(`+0x9c79`) 뿐**이고 `idx`(`+0x9c77`)는
/// 열어 둔 슬롯 번호로 남는다. ⛔ 이 창에서 `idx` 를 쓰지 말 것.
pub const EQUIP_CAND: Grid = Grid {
    x0: 117,
    y0: EQUIP.y0,
    w: EQUIP.w,
    h: EQUIP.h,
    pitch_x: EQUIP.pitch_x,
    pitch_y: EQUIP.pitch_y,
    cols: EQUIP.cols,
    rows: EQUIP.rows,
    pad_x: EQUIP.pad_x,
    pad_y: EQUIP.pad_y,
};

/// 장비 화면의 노란 테두리가 어느 격자의 몇 번 칸에 앉아 있는가.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct EquipMark {
    /// true = 오른쪽 후보 목록([`EQUIP_CAND`]), false = 왼쪽 착용 슬롯([`EQUIP`]).
    pub candidates: bool,
    /// 그 격자 안의 칸 (`row * 3 + col`).
    pub cell: u8,
}

/// 지금 노란 칸 테두리가 앉아 있는 장비 칸 — 없으면 `None`.
///
/// 잉크는 인벤토리 셀 커서와 같은 [`CELL_BOX_INK`] 지만, **크기(16x16)와 칸 원점이
/// 정확히 일치할 것**을 요구하므로 인벤(17x17, 간격 19)과 섞이지 않는다.
/// `status_marker` 와 같은 규약이다 — 워커가 라이브 커서와 대조해서 같을 때만 움직인다.
///
/// ★ 80차-I 부터 **어느 격자인지**도 답한다 — 두 격자는 같은 크기라 원점으로만 갈린다.
pub fn equip_marker(px: &[u32], lw: i32, lh: i32) -> Option<EquipMark> {
    let (ox, oy) = panel_origin(lw, lh)?;
    let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for dy in 0..PANEL_H {
        for dx in 0..PANEL_W {
            let (x, y) = (ox + dx, oy + dy);
            if x < 0 || x >= lw || y < 0 || y >= lh {
                continue;
            }
            if px[(y * lw + x) as usize] & 0x00ff_ffff == CELL_BOX_INK {
                x0 = x0.min(dx);
                y0 = y0.min(dy);
                x1 = x1.max(dx);
                y1 = y1.max(dy);
            }
        }
    }
    if x0 == i32::MAX || x1 - x0 + 1 != EQUIP.w || y1 - y0 + 1 != EQUIP.h {
        return None;
    }
    [(false, &EQUIP), (true, &EQUIP_CAND)].into_iter().find_map(|(candidates, g)| {
        (0..g.cells())
            .find(|i| {
                let (col, row) = (i % g.cols, i / g.cols);
                x0 == g.x0 + g.pitch_x * col && y0 == g.y0 + g.pitch_y * row
            })
            .and_then(|i| u8::try_from(i).ok())
            .map(|cell| EquipMark { candidates, cell })
    })
}

// ── 셸의 SLOT 목록 (80차-C) ──────────────────────────────────────────────────
//
// ★★★ 플레이어 요청 2026-09-05: *"네트워크창고와 게임시작에서 캐릭터 터치로 포커스
// 이동하는거 되어야된다"*. 이 목록은 **캐릭터가 로드되기 전**에 뜨므로 공용 위젯 커서
// (`CHAR+0xa284`)를 읽을 수 없다 — CHAR 자체가 해석이 안 된다(기기 실측: `char=None`).
// 그래서 확인창과 같은 방법을 쓴다: **픽셀이 유일한 창구**다.
//
// 실측(S9, vc247): 온라인 슬롯 목록과 게임시작 슬롯 목록이 **바이트 단위로 같은 위젯**이다.
//   행 dx 32..144 (113 폭), top dy 22 / 39 / 56 (pitch 17), 높이 ~13.
//   안 선택된 행 = `0xcecac5` 단색 (밴드당 84..111 px).
//   선택된 행    = 어두운 갈색 **디더** — `0x4a3d31`/`0x4a3a2b`/`0x4a3929`/`0x4a3c2f` 가
//                  거의 같은 비율로 섞인다. 즉 **단일 잉크로 못 잡는다**.
// ⇒ 커서는 "밝지 않은 행"으로 정의한다. 그 편이 디더에 무관하고, 밝은 쪽은 단색이라
//   임계값이 아주 넓다(0 대 84+).
pub const SLOT_ROW_X0: i32 = 32;
pub const SLOT_ROW_W: i32 = 113;
pub const SLOT_ROW_TOP: i32 = 22;
pub const SLOT_ROW_PITCH: i32 = 17;
pub const SLOT_ROW_H: i32 = 13;
/// 이 게임의 세이브 슬롯 수. 목록은 언제나 셋이다.
pub const SLOT_ROWS: usize = 3;
/// 안 선택된 행의 단색.
pub const SLOT_ROW_LIGHT: u32 = 0x00ce_cac5;
/// 한 행을 "밝다"고 보는 최소 픽셀 수. 실측은 밴드당 850~1110, 커서 행은 0 —
/// 300 은 그 사이 어디에도 안 걸치는 값이다.
pub const SLOT_ROW_LIGHT_MIN: i32 = 300;

/// 셸 SLOT 목록의 세 행과 **커서가 앉은 행**. 그 모양이 아니면 `None`.
///
/// ⛔ 셋 중 정확히 하나만 어두워야 한다. 둘 이상이 어두우면(또는 하나도 없으면) 이 화면이
/// 아니거나 그리는 중이므로 아무것도 돌려주지 않는다 — fail closed.
pub fn slot_rows(px: &[u32], lw: i32, lh: i32) -> Option<(usize, [Box2; SLOT_ROWS])> {
    if lw <= 0 || lh <= 0 || px.len() != (lw as usize) * (lh as usize) {
        return None;
    }
    let (ox, oy) = panel_origin(lw, lh)?;
    let mut boxes = [Box2 { dx: 0, dy: 0, w: 0, h: 0 }; SLOT_ROWS];
    let mut dark = None;
    let mut dark_n = 0;
    for i in 0..SLOT_ROWS {
        let top = SLOT_ROW_TOP + SLOT_ROW_PITCH * i as i32;
        boxes[i] = Box2 { dx: SLOT_ROW_X0, dy: top, w: SLOT_ROW_W, h: SLOT_ROW_H };
        // 위/아래 2px 는 테두리·안티에일리어싱이라 뺀다.
        let mut light = 0;
        for dy in top + 2..top + SLOT_ROW_H - 1 {
            let y = oy + dy;
            if y < 0 || y >= lh {
                return None;
            }
            for dx in SLOT_ROW_X0..SLOT_ROW_X0 + SLOT_ROW_W {
                let x = ox + dx;
                if x >= 0 && x < lw && px[(y as usize) * (lw as usize) + x as usize] & 0x00ff_ffff == SLOT_ROW_LIGHT {
                    light += 1;
                }
            }
        }
        if light < SLOT_ROW_LIGHT_MIN {
            dark = Some(i);
            dark_n += 1;
        }
    }
    (dark_n == 1).then(|| (dark.unwrap(), boxes))
}

// ── 150차 — 아이템 상세보기 (게임 오버레이 페이지 7) ───────────────────────────────
//
// 가방·장비 후보의 동작 팝업 「상세보기」가 여는 창(페인터 `0x4503c`, 키 `0x46340` — CLR 하나만
// 받는다). 150차부터 호스트가 그 **본문 상자**를 스크롤되는 판으로 다시 그린다(`itemnote.rs`:
// 장신구 3번째 옵션 · 「기절 저항 +0」 줄 제거 · 세트 설명). 그 전에 이 창이 정말 떠 있는지를
// 픽셀로 묻는 것이 [`detail_page`] 다. 실측(wie_probe 496x240, vc422 에셋 — 율리케의목걸이와
// 템플러헬름 두 프레임에서 아래 행들이 바이트 단위로 같다):
//   흰 윤곽 #ffffff  dy 15 · 56 (위 상자) · 57 · 157 (본문 상자), dx 1..=173
//   테두리 #7b5952   dy 58..=60 · 155..=156, dx 2..=172 — 그리고 dy 61..=154 의 dx 2·3·172
//   본문 #522d19     dx 4..=171 × dy 61..=154 (61·154 행은 양 끝 한 칸이 테두리) — 게임의
//                    글자는 dy 63 부터이고 마지막 줄(소켓·오브)이 146 에서 끝나므로 dy 62 · 153
//                    두 행은 언제나 통째로 본문색이다.
// ⛔ 호스트가 그린 판(합성된 프레임)에 묻지 말 것 — 스크롤된 호스트 글자가 dy 62·153 에 닿는다. 언제나
// 게임 프레임(`last_frame`)에 묻는다.
/// 상세보기 본문 상자 (패널 상대). `itemnote::BODY_*` 와 같은 값이다(셀프테스트가 묶는다).
pub const DETAIL_BODY: Box2 = Box2 { dx: 4, dy: 61, w: 168, h: 94 };
/// 본문의 채움색.
pub const DETAIL_BODY_INK: u32 = 0x0052_2d19;
/// 본문 상자의 테두리색.
pub const DETAIL_BORDER_INK: u32 = 0x007b_5952;
/// 두 상자의 흰 윤곽.
pub const DETAIL_OUTLINE_INK: u32 = 0x00ff_ffff;
/// 흰 윤곽이 dx 1..=173 을 통째로 긋는 행.
pub const DETAIL_OUTLINE_ROWS: [i32; 4] = [15, 56, 57, 157];
/// 테두리가 dx 2..=172 를 통째로 긋는 행.
pub const DETAIL_BORDER_ROWS: [i32; 5] = [58, 59, 60, 155, 156];
/// 본문이 dx 4..=171 을 통째로 채우는 행 (글자가 닿지 않는다).
pub const DETAIL_CLEAR_ROWS: [i32; 2] = [62, 153];

/// 게임의 상세보기 창(페이지 7)이 떠 있는가. 위 윤곽·테두리·본문 행과 본문 양옆 세로 테두리가
/// **한 픽셀도 빠짐없이** 그 색이어야 한다 — FAIL CLOSED: 하나라도 어긋나면 `false` 이고, 그러면
/// 호스트는 아무것도 덧그리지 않는다.
pub fn detail_page(px: &[u32], lw: i32, lh: i32) -> bool {
    if lw <= 0 || lh <= 0 || px.len() != (lw as usize) * (lh as usize) {
        return false;
    }
    let Some((ox, oy)) = panel_origin(lw, lh) else { return false };
    let is = |dx: i32, dy: i32, want: u32| {
        let (x, y) = (ox + dx, oy + dy);
        x >= 0 && x < lw && y >= 0 && y < lh && px[(y as usize) * (lw as usize) + x as usize] & 0x00ff_ffff == want
    };
    let row = |dy: i32, x0: i32, x1: i32, want: u32| (x0..=x1).all(|dx| is(dx, dy, want));
    let body = DETAIL_BODY;
    DETAIL_OUTLINE_ROWS.iter().all(|&dy| row(dy, 1, 173, DETAIL_OUTLINE_INK))
        && DETAIL_BORDER_ROWS.iter().all(|&dy| row(dy, 2, 172, DETAIL_BORDER_INK))
        && DETAIL_CLEAR_ROWS.iter().all(|&dy| row(dy, body.dx, body.dx + body.w - 1, DETAIL_BODY_INK))
        && (body.dy..body.dy + body.h).all(|dy| {
            is(1, dy, DETAIL_OUTLINE_INK)
                && is(2, dy, DETAIL_BORDER_INK)
                && is(3, dy, DETAIL_BORDER_INK)
                && is(172, dy, DETAIL_BORDER_INK)
                && is(173, dy, DETAIL_OUTLINE_INK)
        })
}
