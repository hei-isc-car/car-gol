from PIL import Image
import struct
import sys

INPUT = sys.argv[1] if len(sys.argv) > 1 else "logo.png"
OUTPUT = sys.argv[2] if len(sys.argv) > 2 else "logo_rgb565.bin"

SIZE = (100, 100)

img = Image.open(INPUT).convert("RGBA")

# Resize to exactly 100x100.
img = img.resize(SIZE, Image.Resampling.LANCZOS)

# Flatten transparency onto black.
background = Image.new("RGBA", SIZE, (0, 0, 0, 255))
img = Image.alpha_composite(background, img).convert("RGB")

with open(OUTPUT, "wb") as f:
    for r, g, b in img.getdata():
        # RGB888 -> RGB565
        rgb565 = (
            ((r & 0xF8) << 8)
            | ((g & 0xFC) << 3)
            | (b >> 3)
        )

        # Big-endian: high byte first.
        f.write(struct.pack(">H", rgb565))

print(f"Wrote {OUTPUT}: {SIZE[0]}x{SIZE[1]} RGB565")
print(f"Size: {SIZE[0] * SIZE[1] * 2} bytes")