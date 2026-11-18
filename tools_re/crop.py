"""Crop the wie_android portrait screenshot down to the emulated 240x320 game area.

Portrait layout (wie_android/src/overlay.rs::layout):
  game_area_h = min(win_w*320/240, win_h*3/5)
  game rect   = contain(0,0,win_w,game_area_h)  -> centered 4:3-ish letterbox
For 1080x2340: game = x=13 y=0 w=1053 h=1404.
"""
import sys, glob
from PIL import Image

for path in sorted(sum((glob.glob(a) for a in sys.argv[1:]), [])):
    im = Image.open(path)
    W, H = im.size
    game_area_h = min(W * 320 // 240, H * 3 // 5)
    scale = min(W / 240.0, game_area_h / 320.0)
    w = int(240 * scale)
    h = int(320 * scale)
    x = (W - w) // 2
    y = (game_area_h - h) // 2
    out = im.crop((x, y, x + w, y + h)).resize((480, 640), Image.NEAREST)
    dst = path.replace('.png', '_g.png')
    out.save(dst)
    print(dst)
