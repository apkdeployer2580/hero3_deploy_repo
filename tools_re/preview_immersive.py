"""가로 몰입 레이아웃 미리보기 — wie_probe 프레임 + 오버레이 기하를 그대로 재현.

`overlay.rs::layout_immersive` / `quick_buttons` / `mask_game_hud` 와 **같은 산식**을
파이썬으로 다시 계산해 실제 게임 프레임 위에 합성한다(렌더러 자체가 아니라 배치 검증용).

usage:
    python preview_immersive.py d:/tmp/probe/menu_rsk/frame_00700_514x240.rgba 2340 1080
"""
import re
import sys

from PIL import Image, ImageDraw

BTN = (0x2C, 0x30, 0x40)
BORDER = (0x70, 0x80, 0xA0)
BAR_TRACK = (0x1C, 0x21, 0x30)
BAR_HP = (0xE0, 0x3A, 0x3A)
BAR_SP = (0x3A, 0x84, 0xE6)
BAR_EXP = (0xF0, 0xC8, 0x28)
QB_SKILL = (0x2A, 0x4A, 0x8A)
QB_ITEM = (0x1F, 0x5A, 0x3A)
KEYS = [["1", "3"], ["5", "9"], ["0", "#"], ["OK", "취소"]]


def main(argv):
    src = argv[1]
    win_w, win_h = int(argv[2]), int(argv[3])
    m = re.search(r'_(\d+)x(\d+)\.rgba$', src)
    lw, lh = int(m.group(1)), int(m.group(2))
    game = Image.frombytes('RGBA', (lw, lh), open(src, 'rb').read())

    # 1:1 fill — logical size is aspect-matched, so this is proportional.
    canvas = game.resize((win_w, win_h), Image.NEAREST).convert('RGBA')
    d = ImageDraw.Draw(canvas, 'RGBA')

    pad = max(min(win_w, win_h) // 110, 3)

    # mask_game_hud: the game's own HUD box = fixed 176x24 logical px, centred,
    # bottom-anchored (measured with wie_probe across 5 logical widths).
    bw = min(win_w * 176 // lw, win_w)
    bh = max(win_h * 24 // lh, 2)
    bx = (win_w - bw) // 2
    d.rectangle([bx, win_h - bh, bx + bw, win_h], fill=(0, 0, 0, 255))

    # right-edge 2x4 translucent key grid
    cell = min(max(win_h // 5, 56), 190)
    gx = max(win_w - (cell * 2 + pad * 2), 0)
    gy = max(win_h - cell * 4, 0) // 2 + win_h // 14
    for r, row in enumerate(KEYS):
        for c, label in enumerate(row):
            x = gx + c * cell + pad
            y = gy + r * cell + pad
            w = h = max(cell - pad * 2, 1)
            d.rectangle([x, y, x + w, y + h], fill=BTN + (140,), outline=BORDER + (210,), width=2)
            tw = len(label) * 7 * max(h // 26, 2)
            d.text((x + w / 2 - tw / 2, y + h / 2 - 8 * max(h // 26, 2)), label,
                   fill=(255, 255, 255, 235))

    # top-left capsule gauges (draw_stats immersive geometry: logical-scaled)
    sp = max(win_w // 48, 3)
    sx, sy = win_w / lw, win_h / lh
    gbw = max(int(96 * sx), 52)
    gbh = min(max(int(6 * sy), 5), 22)
    gap = max(gbh * 3 // 4, 3)
    y = sp
    for frac, col in ((0.72, BAR_HP), (0.45, BAR_SP), (0.30, BAR_EXP)):
        d.rounded_rectangle([sp, y, sp + gbw, y + gbh], radius=gbh // 2, fill=BAR_TRACK + (255,))
        d.rounded_rectangle([sp, y, sp + int(gbw * frac), y + gbh], radius=gbh // 2, fill=col + (255,))
        y += gbh + gap

    # quickbar INSIDE the masked HUD box, centred (quick_buttons immersive branch)
    n = 5
    qs = max(min(bh * 92 // 100, bw // n), 24)
    g = max(qs // 8, 2)
    total = qs * n + g * (n - 1)
    qx = bx + max(bw - total, 0) // 2
    qy = (win_h - bh) + max(bh - qs, 0) // 2
    for i in range(n):
        col = QB_SKILL if i < 2 else QB_ITEM
        d.ellipse([qx, qy, qx + qs, qy + qs], fill=col + (255,), outline=BORDER + (255,), width=2)
        qx += qs + g

    # gear, top-left of the key side (moved to the left in immersive mode)
    gs = min(max(min(win_w, win_h) // 11, 60), 150)
    d.rectangle([pad * 2, pad * 2, pad * 2 + gs, pad * 2 + gs], fill=BTN + (200,),
                outline=BORDER + (255,), width=2)

    out = f'd:/tmp/probe/preview_immersive_{win_w}x{win_h}_from{lw}x{lh}.png'
    canvas.convert('RGB').save(out)
    print('wrote', out, canvas.size, f'(logical {lw}x{lh}, hud box {bw}x{bh})')


if __name__ == '__main__':
    main(sys.argv)
