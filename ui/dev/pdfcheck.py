"""Development only: prints the annotations and page rotations of a PDF as JSON.

Reads the file into memory and closes it at once, so the check holds no handle on the file.
Rectangles come from the annotation vertices (four points per quad), in unrotated page points
with the origin at the top left, the same space as Kurasi's highlight quads.

Usage: python pdfcheck.py <file.pdf>
"""

import json
import sys

import pymupdf as fitz


def rects(annot):
    verts = annot.vertices or []
    out = []
    for i in range(0, len(verts) - 3, 4):
        xs = [p[0] for p in verts[i:i + 4]]
        ys = [p[1] for p in verts[i:i + 4]]
        out.append([round(min(xs), 2), round(min(ys), 2), round(max(xs), 2), round(max(ys), 2)])
    if not out:
        r = annot.rect
        out.append([round(r.x0, 2), round(r.y0, 2), round(r.x1, 2), round(r.y1, 2)])
    return out


def main(path):
    with open(path, "rb") as fh:
        data = fh.read()
    doc = fitz.open(stream=data, filetype="pdf")
    pages = []
    for page in doc:
        annots = []
        for a in page.annots() or []:
            info = a.info or {}
            annots.append({"xref": a.xref, "type": a.type[1], "title": info.get("title", ""),
                           "content": info.get("content", ""), "rects": rects(a),
                           "stroke": list(a.colors.get("stroke") or [])})
        pages.append({"n": page.number + 1, "rotation": page.rotation, "annots": annots})
    doc.close()
    print(json.dumps({"path": path, "pages": pages}, ensure_ascii=False))


if __name__ == "__main__":
    main(sys.argv[1])
