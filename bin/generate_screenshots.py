#!/usr/bin/env python3
import os
import sys
import time
import pty
import select
import termios
import struct
import fcntl
import subprocess
import pyte
from PIL import Image, ImageDraw, ImageFont

COLS = 110
ROWS = 32
FONT_PATH = "/home/ivelot/.local/share/fonts/JetBrainsMono/JetBrainsMonoNerdFont-Regular.ttf"
FONT_SIZE = 26
PAD_X = 36
PAD_Y = 36
BG_COLOR = (13, 15, 24)

ANSI_COLORS = {
    "black": (20, 20, 20),
    "red": (235, 64, 52),
    "green": (80, 250, 123),
    "yellow": (241, 250, 140),
    "blue": (98, 114, 164),
    "magenta": (255, 121, 198),
    "cyan": (139, 233, 253),
    "white": (248, 248, 242),
    "brightblack": (98, 114, 164),
    "brightred": (255, 85, 85),
    "brightgreen": (105, 255, 148),
    "brightyellow": (255, 255, 165),
    "brightblue": (130, 170, 255),
    "brightmagenta": (255, 150, 220),
    "brightcyan": (160, 245, 255),
    "brightwhite": (255, 255, 255),
}

def parse_color(c, is_bg=False):
    if c == "default":
        return BG_COLOR if is_bg else (220, 225, 235)
    if isinstance(c, str):
        if len(c) == 6:
            try:
                r = int(c[0:2], 16)
                g = int(c[2:4], 16)
                b = int(c[4:6], 16)
                return (r, g, b)
            except ValueError:
                pass
        if c.lower() in ANSI_COLORS:
            return ANSI_COLORS[c.lower()]
    return BG_COLOR if is_bg else (220, 225, 235)

def render_screen_to_image(screen, font):
    bbox = font.getbbox("M")
    char_w = bbox[2] - bbox[0] + 1
    char_h = int(FONT_SIZE * 1.55)

    img_w = COLS * char_w + (PAD_X * 2)
    img_h = ROWS * char_h + (PAD_Y * 2)

    img = Image.new("RGB", (img_w, img_h), BG_COLOR)
    draw = ImageDraw.Draw(img)

    for y in range(ROWS):
        for x in range(COLS):
            cell = screen.buffer[y][x]
            fg = parse_color(cell.fg, is_bg=False)
            bg = parse_color(cell.bg, is_bg=True)

            px = PAD_X + x * char_w
            py = PAD_Y + y * char_h

            if bg != BG_COLOR:
                draw.rectangle([px, py, px + char_w, py + char_h], fill=bg)

            if cell.data and cell.data != " ":
                draw.text((px, py + 2), cell.data, font=font, fill=fg)

    return img

def set_terminal_size(fd, cols, rows):
    winsize = struct.pack("HHHH", rows, cols, 0, 0)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, winsize)

def capture_sessions(binary_path, out_dir):
    os.makedirs(out_dir, exist_ok=True)
    font = ImageFont.truetype(FONT_PATH, FONT_SIZE)

    targets = [
        ("overview.png", []),
        ("overview_detailed.png", ["."]),
        ("network.png", ["2"]),
        ("bluetooth.png", ["3"]),
        ("tab3_bluetooth.png", ["3"]),
        ("storage.png", ["4"]),
        ("tab4_disk_analyzer.png", ["4", "a"]),
        ("audio_mixer.png", ["5"]),
        ("tab5_audio.png", ["5"]),
        ("displays.png", ["6"]),
        ("tab6_display.png", ["6"]),
        ("config_modal.png", ["c"]),
    ]

    for filename, key_seq in targets:
        print(f"[*] Capturing {filename} in English (2x HD) with keys {key_seq}...")
        master, slave = pty.openpty()
        set_terminal_size(slave, COLS, ROWS)

        screen = pyte.Screen(COLS, ROWS)
        stream = pyte.ByteStream(screen)

        env = os.environ.copy()
        env["TERM"] = "xterm-256color"
        env["COLORTERM"] = "truecolor"
        env["LANG"] = "en_US.UTF-8"
        env["LC_ALL"] = "en_US.UTF-8"
        env["LC_MESSAGES"] = "en_US.UTF-8"
        env["HAL9001_CONFIG"] = "/nonexistent/config.toml"

        p = subprocess.Popen(
            [binary_path],
            stdin=slave,
            stdout=slave,
            stderr=slave,
            close_fds=True,
            env=env,
        )
        os.close(slave)

        time.sleep(0.4)
        os.write(master, b" ") # pass splash screen
        time.sleep(0.4)

        for k in key_seq:
            os.write(master, k.encode("utf-8"))
            time.sleep(0.35)

        deadline = time.time() + 1.2
        while time.time() < deadline:
            r, _, _ = select.select([master], [], [], 0.1)
            if r:
                try:
                    data = os.read(master, 4096)
                    if not data:
                        break
                    stream.feed(data)
                except OSError:
                    break

        try:
            os.write(master, b"q")
            p.terminate()
            p.wait(timeout=0.5)
        except Exception:
            pass
        os.close(master)

        img = render_screen_to_image(screen, font)
        out_path = os.path.join(out_dir, filename)
        img.save(out_path, "PNG", optimize=True)
        print(f"    Saved: {out_path} ({img.width}x{img.height})")

if __name__ == "__main__":
    bin_path = "/home/ivelot/Projetos/firstmate/projects/hall-9001/target/release/hal9001"
    output_dir = "/home/ivelot/Projetos/firstmate/projects/hall-9001/assets/screenshots"
    capture_sessions(bin_path, output_dir)
    print("[+] All high-res English screenshots generated successfully!")
