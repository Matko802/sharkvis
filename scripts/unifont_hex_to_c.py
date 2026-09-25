import sys


def parse_hex(path):
    glyphs = []
    with open(path, "r", encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not line or ":" not in line:
                continue
            cp_s, bmp = line.split(":", 1)
            try:
                cp = int(cp_s, 16)
            except ValueError:
                continue
            if cp > 0xFFFF:
                continue
            bmp = bmp.strip()
            if len(bmp) == 32:
                w = 8
                rows = [int(bmp[i:i + 2], 16) for i in range(0, 32, 2)]
                packed = []
                for r in rows:
                    packed.append(r)
                    packed.append(0)
            elif len(bmp) == 64:
                w = 16
                packed = [int(bmp[i:i + 2], 16) for i in range(0, 64, 2)]
            else:
                continue
            glyphs.append((cp, w, packed))
    glyphs.sort()
    return glyphs


def main():
    src, out_c, out_h = sys.argv[1], sys.argv[2], sys.argv[3]
    glyphs = parse_hex(src)
    with open(out_h, "w", encoding="utf-8") as f:
        f.write("#ifndef SHARKVIS_UNIFONT_DATA_H\n")
        f.write("#define SHARKVIS_UNIFONT_DATA_H\n")
        f.write("#include <stddef.h>\n")
        f.write("#include <stdint.h>\n")
        f.write("typedef struct { uint32_t cp; uint8_t w; uint8_t rows[32]; }\n")
        f.write("    unifont_glyph_t;\n")
        f.write("extern const unifont_glyph_t unifont_glyphs[];\n")
        f.write("extern const size_t unifont_glyph_count;\n")
        f.write("#endif\n")
    with open(out_c, "w", encoding="utf-8") as f:
        f.write('#include "unifont_data.h"\n')
        f.write("const unifont_glyph_t unifont_glyphs[] = {\n")
        for cp, w, packed in glyphs:
            cells = ",".join(str(b) for b in packed)
            f.write("{%d,%d,{%s}}," % (cp, w, cells))
            f.write("\n")
        f.write("};\n")
        f.write("const size_t unifont_glyph_count = %d;\n" % len(glyphs))


main()
