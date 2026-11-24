"""mos_打光 v1.0.0 协议、拼贴防穿帮、遮罩缩放、黑白/四色渐变叠加与 EffectCraft 端到端实测。

用法：
  python tools/test_helper.py [effectcraft-helper-mos-rimlight.exe 路径]
"""

import array
import json
import os
import queue
import shutil
import struct
import subprocess
import sys
import tempfile
import threading
import zlib

PROTO = 1
EFFECT = "mos.rimlight"
HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT_DIR = os.path.join(HERE, "tests", "out")
EC_CLI = r"D:\暂时\effectcraft-full-featured\effectcraft-cli.exe"
REF_PROJ = r"D:\暂时\5\打光.ecproj"


class Helper:
    def __init__(self, exe, env_extra=None):
        env = dict(os.environ)
        if env_extra:
            env.update(env_extra)
        self.proc = subprocess.Popen(
            [exe, "--effectcraft-helper"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            bufsize=0,
            env=env,
        )
        self.q = queue.Queue()
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        for raw in self.proc.stdout:
            line = raw.decode("utf-8", "replace").strip()
            if not line:
                continue
            if line.startswith("@ec-helper "):
                line = line[len("@ec-helper "):]
            try:
                self.q.put(json.loads(line))
            except ValueError:
                self.q.put({"op": "@@raw", "line": line})

    def send(self, obj):
        self.proc.stdin.write((json.dumps(obj) + "\n").encode("utf-8"))
        self.proc.stdin.flush()

    def recv(self, timeout=15.0):
        return self.q.get(timeout=timeout)

    def close(self):
        try:
            self.send({"op": "shutdown"})
        except OSError:
            pass
        try:
            self.proc.stdin.close()
        except OSError:
            pass
        try:
            self.proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.proc.kill()


def make_params(w=640, h=360, **overrides):
    # Matches flat indices 0..66 (67 f64s) in helper/src/rimlight.rs
    defaults = [
        120.0, 18.0, 2.5, -1.5,                        # 0..3: top-level
        0.0, 0.0, 8.0, -16.0, 100.0, 0.0, 0.0,         # 4..10: tile, offset_mode, ox, oy, scale, feather, choke
        0.0, 1.0,                                      # 11..12: base_offset, base_gamma
        1.0, 0.0, 1.0, 0.95, 0.85, 1.0, 100.0,         # 13..19: rim_gamma, tint_mix, rim_color, rim_opacity
        0.0, 0.0, 0.0, 60.0,                           # 20..23: overlay_enable, type, blend, opacity
        w * 0.5, h * 0.15, w * 0.5, h * 0.85, 0.0, 0.0,# 24..29: bw_start, bw_end, bw_shape, bw_invert
        w * 0.15, h * 0.15, 1.0, 0.85, 0.65, 1.0,      # 30..35: c4_p1, col1
        w * 0.85, h * 0.15, 0.35, 0.72, 1.0, 1.0,      # 36..41: c4_p2, col2
        w * 0.15, h * 0.85, 0.75, 0.35, 0.95, 1.0,     # 42..47: c4_p3, col3
        w * 0.85, h * 0.85, 0.12, 0.16, 0.32, 1.0,     # 48..53: c4_p4, col4
        100.0,                                         # 54: c4_blend
        1.0, 60.0, 21.0, 100.0, 1.0, 0.0,              # 55..60: glow_enable, threshold, radius, intensity, outside, tint_mix
        0.75, 0.90, 1.0, 1.0,                          # 61..64: glow_color
        0.0, 100.0,                                    # 65..66: output_mode, mix
    ]
    idx = {
        "light_angle": 0, "edge_distance": 1, "rim_exposure": 2, "base_exposure": 3,
        "tile_mode": 4, "offset_mode": 5, "offset_x": 6, "offset_y": 7, "mask_scale": 8,
        "mask_feather": 9, "mask_choke": 10,
        "base_offset": 11, "base_gamma": 12,
        "rim_gamma": 13, "rim_tint_mix": 14, "rim_opacity": 19,
        "overlay_enable": 20, "overlay_type": 21, "overlay_blend": 22, "overlay_opacity": 23,
        "bw_shape": 28, "bw_invert": 29, "c4_blend": 54,
        "glow_enable": 55, "glow_threshold": 56, "glow_radius": 57, "glow_intensity": 58,
        "glow_outside": 59, "glow_tint_mix": 60,
        "output_mode": 65, "mix": 66,
    }
    p = list(defaults)
    for k, v in overrides.items():
        p[idx[k]] = float(v)
    return p


def write_frame(fh, offset, floats):
    arr = array.array("f", floats)
    if sys.byteorder != "little":
        arr.byteswap()
    fh.seek(offset)
    fh.write(arr.tobytes())
    fh.flush()


def read_frame(fh, offset, n_floats):
    fh.seek(offset)
    raw = fh.read(n_floats * 4)
    arr = array.array("f")
    arr.frombytes(raw)
    if sys.byteorder != "little":
        arr.byteswap()
    return arr


def save_png(path, w, h, premul_rgba, bg=(0.14, 0.42, 0.85)):
    rows = bytearray()
    for y in range(h):
        rows.append(0)
        row_off = y * w * 4
        for x in range(w):
            o = row_off + x * 4
            r = premul_rgba[o]
            g = premul_rgba[o + 1]
            b = premul_rgba[o + 2]
            a = max(0.0, min(1.0, premul_rgba[o + 3]))
            inv = 1.0 - a
            cr = max(0.0, min(1.0, r + bg[0] * inv))
            cg = max(0.0, min(1.0, g + bg[1] * inv))
            cb = max(0.0, min(1.0, b + bg[2] * inv))
            rows.append(int(round(cr * 255)))
            rows.append(int(round(cg * 255)))
            rows.append(int(round(cb * 255)))
            rows.append(255)

    def chunk(tag, data):
        crc = zlib.crc32(tag + data) & 0xFFFFFFFF
        return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", crc)

    ihdr = struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0)
    png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) + chunk(b"IDAT", zlib.compress(bytes(rows), 6)) + chunk(b"IEND", b"")
    with open(path, "wb") as f:
        f.write(png)


def make_edge_touching_character(w, h):
    """Draw a silhouette touching top & bottom edges to test Mirror Tiling anti-leak."""
    buf = [0.0] * (w * h * 4)
    cx = w * 0.5
    for y in range(h):
        # Wide column touching y=0 and y=h-1, with curved waist
        half = w * (0.18 + 0.06 * ((y - h * 0.5) / (h * 0.5)) ** 2)
        for x in range(w):
            if abs(x - cx) <= half:
                o = (y * w + x) * 4
                buf[o] = 0.25
                buf[o + 1] = 0.40
                buf[o + 2] = 0.58
                buf[o + 3] = 1.0
    return buf


def test_protocol_and_tiling(exe):
    os.makedirs(OUT_DIR, exist_ok=True)
    with tempfile.NamedTemporaryFile(delete=False) as tf:
        frames_path = tf.name

    try:
        for lang_code, exp_name, exp_cat in [("zh", "mos_打光", "MOS 视效"), ("en", "mos_rimlight", "MOS VFX")]:
            h = Helper(exe, {"MOS_LANG": lang_code})
            h.send({"op": "hello", "version": PROTO, "frames": frames_path})
            rep = h.recv()
            assert rep["op"] == "hello" and rep["name"] == exp_name, rep
            h.send({"op": "describe"})
            desc = h.recv()
            m = desc["effects"][0]["manifest"]
            by_id = {p["id"]: p for p in m["params"]}
            assert "tile_mode" in by_id and "mask_scale" in by_id and "overlay_enable" in by_id and "glow_enable" in by_id
            h.close()
        print("[OK] 中英双语 manifest、拼贴、遮罩缩放、渐变叠加与发光开关校验通过")

        h = Helper(exe, {"MOS_LANG": "zh"})
        h.send({"op": "hello", "version": PROTO, "frames": frames_path})
        assert h.recv()["op"] == "hello"
        h.send({"op": "describe"})
        h.recv()

        # Verify Mirror Tiling prevents top-edge leak when light comes from top (angle=270)
        w, hh = 480, 320
        base = make_edge_touching_character(w, hh)
        with open(frames_path, "r+b") as fh:
            # Mirror ON -> top-center pixel (y=2, x=w/2) should have NO false rim leak
            write_frame(fh, 0, base)
            p_mirror = make_params(w, hh, light_angle=270, edge_distance=20, tile_mode=0, output_mode=2, glow_enable=0)
            h.send({"op": "render", "effect": EFFECT, "instance": 1, "time": 0.0,
                    "frame": {"offset": 0, "width": w, "height": hh, "origin": [0.0, 0.0], "scale": 1.0},
                    "layer_size": [float(w), float(hh)], "flat": p_mirror})
            assert h.recv()["op"] == "done"
            out_m = read_frame(fh, 0, w * hh * 4)
            top_leak_mirror = out_m[(2 * w + w // 2) * 4 + 3]

            # Tile OFF (None) -> top-center pixel leaks!
            write_frame(fh, 0, base)
            p_none = make_params(w, hh, light_angle=270, edge_distance=20, tile_mode=3, output_mode=2, glow_enable=0)
            h.send({"op": "render", "effect": EFFECT, "instance": 1, "time": 0.0,
                    "frame": {"offset": 0, "width": w, "height": hh, "origin": [0.0, 0.0], "scale": 1.0},
                    "layer_size": [float(w), float(hh)], "flat": p_none})
            assert h.recv()["op"] == "done"
            out_n = read_frame(fh, 0, w * hh * 4)
            top_leak_none = out_n[(2 * w + w // 2) * 4 + 3]

            assert top_leak_mirror < 0.01 and top_leak_none > 0.9, (top_leak_mirror, top_leak_none)
            print(f"[OK] 镜像拼贴防边界穿帮校验通过 (Mirror 顶边泄漏={top_leak_mirror:.4f}, 关闭拼贴泄漏={top_leak_none:.4f})")

            # Sample with 4-color gradient overlay beneath rim & glow + mask scale 96%
            write_frame(fh, 0, base)
            p_c4 = make_params(w, hh, light_angle=135, edge_distance=14, mask_scale=96, overlay_enable=1, overlay_type=1, overlay_blend=0, overlay_opacity=75, glow_enable=1)
            h.send({"op": "render", "effect": EFFECT, "instance": 1, "time": 0.0,
                    "frame": {"offset": 0, "width": w, "height": hh, "origin": [0.0, 0.0], "scale": 1.0},
                    "layer_size": [float(w), float(hh)], "flat": p_c4})
            assert h.recv()["op"] == "done"
            save_png(os.path.join(OUT_DIR, "01_tiling_scale_4color.png"), w, hh, read_frame(fh, 0, w * hh * 4))
            print("[OK] 渲染样张 01_tiling_scale_4color.png")

        h.close()
    finally:
        if os.path.exists(frames_path):
            os.remove(frames_path)


def test_reference_project_e2e():
    if not (os.path.exists(EC_CLI) and os.path.exists(REF_PROJ)):
        print("[SKIP] 缺少 EffectCraft CLI 或参考工程")
        return

    tmp_proj = os.path.join(OUT_DIR, "sayaka_mos_rimlight.ecproj")
    shutil.copy2(REF_PROJ, tmp_proj)
    out_classic = os.path.join(OUT_DIR, "02_sayaka_classic_rimlight.png")
    out_4color = os.path.join(OUT_DIR, "03_sayaka_4color_underglow.png")

    env = dict(os.environ)
    env["EFFECTCRAFT_HELPERS"] = "1"

    # Disable original manual precomp (#1) & original exposure on sayaka (#2), then apply single mos.rimlight!
    cmds = [
        {"command": "effect.helpers.scan", "params": {}},
        {"command": "layer.setSwitch", "params": {"comp": "Comp 1", "layer": "#1", "switch": "video", "value": False}},
        {"command": "prop.set", "params": {"comp": "Comp 1", "layer": "#2", "path": "effects/#1/master/exposure", "value": 0.0}},
        {"command": "effect.apply", "params": {"comp": "Comp 1", "layer": "#2", "effect": EFFECT}},
        {"command": "prop.set", "params": {"comp": "Comp 1", "layer": "#2", "path": "effects/#2/base_exposure", "value": -3.2}},
        {"command": "prop.set", "params": {"comp": "Comp 1", "layer": "#2", "path": "effects/#2/rim_exposure", "value": 2.8}},
        {"command": "prop.set", "params": {"comp": "Comp 1", "layer": "#2", "path": "effects/#2/edge_distance", "value": 18.0}},
        {"command": "prop.set", "params": {"comp": "Comp 1", "layer": "#2", "path": "effects/#2/light_angle", "value": 116.0}},
        {"command": "renderQueue.add", "params": {"comp": "Comp 1", "format": "png", "timeSpan": "custom", "start": 0.0, "duration": 0.034, "output": out_classic.replace("\\", "/")}},
        {"command": "renderQueue.render", "params": {"wait": True}},
    ]
    proc = subprocess.run(
        [EC_CLI, "--project", tmp_proj, "--save", "run", "--commands", "-"],
        input=json.dumps(cmds).encode("utf-8"),
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        env=env,
        timeout=60,
    )
    assert proc.returncode == 0, proc.stderr.decode("utf-8", "replace")
    print(f"[OK] 参考工程沙耶香单插件打光渲染完成: {out_classic}")

    # Render second variant with 4-color gradient overlay beneath glow + slight mask scale!
    cmds2 = [
        {"command": "effect.helpers.scan", "params": {}},
        {"command": "prop.set", "params": {"comp": "Comp 1", "layer": "#2", "path": "effects/#2/u4e2du5c42u6e10u53d8u53e0u52a0/overlay_enable", "value": True}},
        {"command": "prop.set", "params": {"comp": "Comp 1", "layer": "#2", "path": "effects/#2/u4e2du5c42u6e10u53d8u53e0u52a0/overlay_type", "value": 1}},
        {"command": "prop.set", "params": {"comp": "Comp 1", "layer": "#2", "path": "effects/#2/u4e2du5c42u6e10u53d8u53e0u52a0/overlay_opacity", "value": 65.0}},
        {"command": "prop.set", "params": {"comp": "Comp 1", "layer": "#2", "path": "effects/#2/u906eu7f69u4e0eu62fcu8d34/mask_scale", "value": 98.5}},
        {"command": "renderQueue.remove", "params": {"index": 1}},
        {"command": "renderQueue.add", "params": {"comp": "Comp 1", "format": "png", "timeSpan": "custom", "start": 0.0, "duration": 0.034, "output": out_4color.replace("\\", "/")}},
        {"command": "renderQueue.render", "params": {"wait": True}},
    ]
    proc2 = subprocess.run(
        [EC_CLI, "--project", tmp_proj, "run", "--commands", "-"],
        input=json.dumps(cmds2).encode("utf-8"),
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        env=env,
        timeout=60,
    )
    assert proc2.returncode == 0, proc2.stderr.decode("utf-8", "replace")
    print(f"[OK] 参考工程沙耶香四色渐变+遮罩缩放样张完成: {out_4color}")


if __name__ == "__main__":
    exe = sys.argv[1] if len(sys.argv) > 1 else os.path.join(HERE, "dist", "effectcraft-helper-mos-rimlight.exe")
    test_protocol_and_tiling(exe)
    test_reference_project_e2e()
    print("\n全部测试通过！")
