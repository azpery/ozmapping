#!/usr/bin/env python3
"""Prépare la tête 3D du visuel « Visage qui parle » à partir des fichiers de MakeHuman (CC0).

Usage : python3 scripts/bake-face.py <dossier des sources> [assets/face]

Le dossier des sources contient, téléchargés tels quels :
  3dobjs/base.obj                      maillage de base (dépôt makehumancommunity/makehuman, data/)
  targets/*.target                     cibles macrodetails (universal-*, caucasian-*) et expression/units/caucasian
  prox/{high-poly,eyebrow001,eyelashes01,teeth_base}.{obj,mhclo}
  tex/*.png                            peaux « lightskinned », yeux, dents, sourcils, cils

Sources : https://github.com/makehumancommunity/makehuman (makehuman/data : 3dobjs, targets, eyes)
et http://download.tuxfamily.org/makehuman/assets/1.1/base/ (skins/textures, eyes/materials, teeth,
eyebrows/eyebrow001, eyelashes/eyelashes01).

Sortie : head.bin (maillage de la tête, cibles de déformation) et les textures recadrées.
Dépendances : Python 3 seul, et ffmpeg pour les images.
"""
import math
import os
import struct
import subprocess
import sys

SRC = sys.argv[1]
OUT = sys.argv[2] if len(sys.argv) > 2 else "assets/face"
os.makedirs(OUT, exist_ok=True)

# Tout ce qui est au-dessus de cette hauteur (base du cou) est gardé.
NECK_Y = 5.95
# Matières, dans l'ordre de dessin (les deux dernières sont transparentes).
SKIN, FLAT, EYE, TEETH, TONGUE, MOUTH, BROWS, LASHES = range(8)
# Zone de la texture de peau où est peint l'intérieur de la bouche.
MOUTH_UV = (0.74, 0.94, 0.0, 0.14)
# Les dents du haut dépassent un peu de la lèvre ; celles du bas suivent la mâchoire d'aussi près
# que la lèvre (dans les fichiers d'origine, elles restent à mi-chemin et se découvrent en entier).
UPPER_TEETH_DROP = 0.035
LOWER_TEETH_FOLLOW = 1.35
# La langue, presque immobile à l'origine, descend avec les dents du bas (part de leur mouvement).
TONGUE_FOLLOW = 0.85


def read_obj(path):
    verts, uvs, faces, group = [], [], [], None
    for line in open(path):
        p = line.split()
        if not p:
            continue
        if p[0] == "v":
            verts.append(tuple(map(float, p[1:4])))
        elif p[0] == "vt":
            uvs.append(tuple(map(float, p[1:3])))
        elif p[0] == "g":
            group = p[1]
        elif p[0] == "f":
            faces.append((group, [tuple(int(x) - 1 for x in q.split("/")[:2]) for q in p[1:]]))
    return verts, uvs, faces


def read_target(path):
    out = {}
    for line in open(path):
        if line.startswith("#") or not line.strip():
            continue
        p = line.split()
        out[int(p[0])] = tuple(map(float, p[1:4]))
    return out


def read_mhclo(path):
    """Attaches d'un accessoire : pour chaque sommet, trois sommets de base, leurs poids, un décalage."""
    scales, binds, on = {}, [], False
    for line in open(path):
        p = line.split()
        if not p or p[0].startswith("#"):
            continue
        if p[0] in ("x_scale", "y_scale", "z_scale"):
            scales[p[0][0]] = (int(p[1]), int(p[2]), float(p[3]))
        elif p[0] == "verts":
            on = True
        elif on and (p[0][0].isdigit() or p[0][0] == "-"):
            if len(p) == 1:
                binds.append(((int(p[0]),) * 3, (1.0, 0.0, 0.0), (0.0, 0.0, 0.0)))
            else:
                binds.append((tuple(map(int, p[0:3])), tuple(map(float, p[3:6])), tuple(map(float, p[6:9]))))
        elif on:
            on = False
    return scales, binds


base_v, base_vt, base_f = read_obj(f"{SRC}/3dobjs/base.obj")

positions = []  # positions de repos
sources = []  # pour chaque position : [(sommet de base, poids)], d'où viennent ses déformations
flags = []  # 1 = crâne ou visage (sert à cadrer la tête)
verts = []  # sommets dessinés : (position, u, v, matière)
tris = {m: [] for m in range(8)}
vert_ids = {}


def add_vertex(pos, u, v, mat):
    key = (pos, u, v, mat)
    if key not in vert_ids:
        vert_ids[key] = len(verts)
        verts.append(key)
    return vert_ids[key]


def add_face(ids, mat):
    for k in range(1, len(ids) - 1):
        tris[mat] += [ids[0], ids[k], ids[k + 1]]


# Peau : la grande île de la texture (visage, crâne, cou) est recadrée ; le reste (oreilles)
# prend une couleur de peau unie.
head_faces = [f for g, f in base_f if g == "body" and all(base_v[i][1] > NECK_Y for i, _ in f)]
island = [base_vt[t] for f in head_faces for _, t in f if base_vt[t][0] > 0.55 and base_vt[t][1] < 0.85]
u0, u1 = min(u for u, _ in island) - 0.004, min(max(u for u, _ in island) + 0.004, 1.0)
v0, v1 = max(min(v for _, v in island) - 0.004, 0.0), max(v for _, v in island) + 0.004
print(f"île du visage : u {u0:.3f}..{u1:.3f}, v {v0:.3f}..{v1:.3f}")

jaw = [i for g, f in base_f if g == "joint-jaw" for i, _ in f]
chin_point = [sum(base_v[i][k] for i in jaw) / len(jaw) for k in range(3)]

# Faces d'origine : (sommets de base, coordonnées de texture, matière).
quads = []
for f in head_faces:
    inside = all(u0 <= base_vt[t][0] <= u1 and v0 <= base_vt[t][1] <= v1 for _, t in f)
    mouth = all(MOUTH_UV[0] < base_vt[t][0] < MOUTH_UV[1] and MOUTH_UV[2] <= base_vt[t][1] < MOUTH_UV[3] for _, t in f)
    mat = MOUTH if mouth else SKIN if inside else FLAT
    uvs = [((base_vt[t][0] - u0) / (u1 - u0), 1.0 - (base_vt[t][1] - v0) / (v1 - v0)) if inside else (0.0, 0.0) for _, t in f]
    quads.append(([i for i, _ in f], uvs, mat))
for g, f in base_f:
    if g == "helper-tongue":
        quads.append(([i for i, _ in f], [(0.0, 0.0)] * 4, TONGUE))


def combine(parts):
    """Somme pondérée de points, chacun donné par ses poids sur les sommets de base."""
    out = {}
    for w, src in parts:
        for i, x in src.items():
            out[i] = out.get(i, 0.0) + w * x
    return out


def subdivide(quads):
    """Un pas de Catmull-Clark : chaque face en donne quatre, la surface s'arrondit.

    Les nouveaux points sont des moyennes pondérées des sommets de base : les cibles de
    déformation s'y appliquent donc telles quelles.
    """
    edges, vert_faces = {}, {}
    for fi, (ids, _, _) in enumerate(quads):
        for k in range(4):
            a, b = ids[k], ids[(k + 1) % 4]
            edges.setdefault((min(a, b), max(a, b)), []).append(fi)
            vert_faces.setdefault(a, []).append(fi)
    face_pt = [combine([(0.25, {i: 1.0}) for i in ids]) for ids, _, _ in quads]
    edge_pt, vert_edges = {}, {}
    for (a, b), fs in edges.items():
        vert_edges.setdefault(a, []).append((a, b))
        vert_edges.setdefault(b, []).append((a, b))
        if len(fs) == 2:
            edge_pt[(a, b)] = combine([(0.25, {a: 1.0}), (0.25, {b: 1.0}), (0.25, face_pt[fs[0]]), (0.25, face_pt[fs[1]])])
        else:
            edge_pt[(a, b)] = {a: 0.5, b: 0.5}
    vert_pt = {}
    for v, fs in vert_faces.items():
        es = vert_edges[v]
        border = [e for e in es if len(edges[e]) != 2]
        if len(border) == 2:
            others = [e[0] if e[1] == v else e[1] for e in border]
            vert_pt[v] = combine([(0.75, {v: 1.0}), (0.125, {others[0]: 1.0}), (0.125, {others[1]: 1.0})])
        elif border or len(fs) != len(es):
            vert_pt[v] = {v: 1.0}
        else:
            n = len(fs)
            vert_pt[v] = combine(
                [(1.0 / n / n, face_pt[fi]) for fi in fs]
                + [(2.0 / n / n, {e[0]: 0.5, e[1]: 0.5}) for e in es]
                + [((n - 3.0) / n, {v: 1.0})]
            )
    ids_of = {}

    def point(key, weights):
        if key not in ids_of:
            ids_of[key] = len(positions)
            p = tuple(sum(w * base_v[i][k] for i, w in weights.items()) for k in range(3))
            positions.append(p)
            sources.append(list(weights.items()))
            flags.append(1 if p[1] > chin_point[1] else 0)
        return ids_of[key]

    def mid(p, q):
        return (0.5 * (p[0] + q[0]), 0.5 * (p[1] + q[1]))

    for fi, (ids, uvs, mat) in enumerate(quads):
        center = point(("f", fi), face_pt[fi])
        center_uv = (sum(u for u, _ in uvs) / 4.0, sum(v for _, v in uvs) / 4.0)
        for k in range(4):
            nxt, prv = (k + 1) % 4, (k + 3) % 4
            e_next = (min(ids[k], ids[nxt]), max(ids[k], ids[nxt]))
            e_prev = (min(ids[k], ids[prv]), max(ids[k], ids[prv]))
            corners = [
                (point(("v", ids[k]), vert_pt[ids[k]]), uvs[k]),
                (point(("e",) + e_next, edge_pt[e_next]), mid(uvs[k], uvs[nxt])),
                (center, center_uv),
                (point(("e",) + e_prev, edge_pt[e_prev]), mid(uvs[k], uvs[prv])),
            ]
            add_face([add_vertex(p, uv[0], uv[1], mat) for p, uv in corners], mat)
    return ids_of


ids_of = subdivide(quads)
chin = ids_of[("v", min({i for ids, _, m in quads if m != TONGUE for i in ids}, key=lambda i: math.dist(base_v[i], chin_point)))]
# Couleur unie : celle de la joue.
cheek = min((v for v in verts if v[3] == SKIN), key=lambda v: math.dist(positions[v[0]], (0.55, 6.85, 1.2)))
flat_uv = (cheek[1], cheek[2])


def scale_of(ref):
    a, b, d = ref
    return lambda axis: abs(base_v[a][axis] - base_v[b][axis]) / d


upper_teeth = {i for g, f in base_f if g == "helper-upper-teeth" for i, _ in f}
lower_teeth = {i for g, f in base_f if g == "helper-lower-teeth" for i, _ in f}
tongue = {i for g, f in base_f if g == "helper-tongue" for i, _ in f}
eye_ranges = []
for name, mat in (("high-poly", EYE), ("teeth_base", TEETH), ("eyebrow001", BROWS), ("eyelashes01", LASHES)):
    pv, pvt, pf = read_obj(f"{SRC}/prox/{name}.obj")
    scales, binds = read_mhclo(f"{SRC}/prox/{name}.mhclo")
    assert len(binds) == len(pv), (name, len(binds), len(pv))
    sc = [scale_of(scales[a])(k) for k, a in enumerate("xyz")]
    first = len(positions)
    for (ids, w, off) in binds:
        p = [sum(w[j] * base_v[ids[j]][k] for j in range(3)) + off[k] * sc[k] for k in range(3)]
        if ids[0] in upper_teeth:
            p[1] -= UPPER_TEETH_DROP
        positions.append(tuple(p))
        sources.append([(ids[j], w[j]) for j in range(3) if w[j] != 0.0])
        flags.append(0)
    if mat == EYE:
        # Les deux yeux sont dans le même fichier : on les sépare pour les faire tourner chacun.
        left = [first + k for k in range(len(pv)) if positions[first + k][0] > 0]
        right = [first + k for k in range(len(pv)) if positions[first + k][0] <= 0]
        eye_ranges = [left, right]
    for _, f in pf:
        add_face([add_vertex(first + i, pvt[t][0], 1.0 - pvt[t][1], mat) for i, t in f], mat)

# Cibles de déformation, limitées aux positions gardées.
morphs = []
for fname in sorted(os.listdir(f"{SRC}/targets")):
    if not fname.endswith(".target"):
        continue
    name = fname[: -len(".target")]
    if name.startswith("universal-"):
        _, sex, age, _, weight = name.split("-")
        name = f"u-{sex}-{age}-{weight.replace('weight', '')}"
    target = read_target(f"{SRC}/targets/{fname}")
    if name == "mouth-open":
        moved = [target[i] for i in lower_teeth if i in target]
        jaw_move = [sum(d[k] for d in moved) / len(moved) * LOWER_TEETH_FOLLOW for k in range(3)]
        target = {i: tuple(x * LOWER_TEETH_FOLLOW for x in d) if i in lower_teeth else d for i, d in target.items()}
        for i in tongue:
            d = target.get(i, (0.0, 0.0, 0.0))
            # Le fond de la langue (vers la gorge) bouge moins que sa pointe.
            tip = min(max((base_v[i][2] - 0.75) / 0.45, 0.0), 1.0)
            target[i] = tuple(d[k] + (jaw_move[k] * TONGUE_FOLLOW - d[k]) * tip for k in range(3))
    sparse = []
    for p, src in enumerate(sources):
        d = [0.0, 0.0, 0.0]
        for i, w in src:
            if i in target:
                for k in range(3):
                    d[k] += w * target[i][k]
        if any(abs(x) > 1e-5 for x in d):
            sparse.append((p, d))
    morphs.append((name, sparse))

order = [SKIN, FLAT, EYE, TEETH, TONGUE, MOUTH, BROWS, LASHES]
print("triangles par matière :", [len(tris[m]) // 3 for m in order])
with open(f"{OUT}/head.bin", "wb") as out:
    out.write(b"OZFACE02")
    out.write(struct.pack("<IIII", len(positions), len(verts), len(morphs), chin))
    out.write(struct.pack("<ff", *flat_uv))
    for p in positions:
        out.write(struct.pack("<fff", *p))
    out.write(bytes(flags))
    for v in verts:
        out.write(struct.pack("<IffB", *v))
    for m in order:
        out.write(struct.pack("<I", len(tris[m])))
        out.write(struct.pack(f"<{len(tris[m])}I", *tris[m]))
    for eye in eye_ranges:
        out.write(struct.pack("<I", len(eye)))
        out.write(struct.pack(f"<{len(eye)}I", *eye))
    for name, sparse in morphs:
        raw = name.encode()
        out.write(struct.pack("<B", len(raw)) + raw)
        out.write(struct.pack("<I", len(sparse)))
        for p, d in sparse:
            out.write(struct.pack("<Ifff", p, *d))
print(f"{len(positions)} positions, {len(verts)} sommets, {sum(len(t) for t in tris.values()) // 3} triangles, {len(morphs)} cibles")
print("taille :", os.path.getsize(f"{OUT}/head.bin"))


def ffmpeg(src, dst, vf):
    subprocess.run(["ffmpeg", "-loglevel", "error", "-y", "-i", src, "-vf", vf, "-q:v", "3", dst], check=True)


# Peaux : recadrées sur l'île du visage (la texture a v = 0 en bas).
size = 2048
x, w = round(u0 * size), round((u1 - u0) * size)
y, h = round((1.0 - v1) * size), round((v1 - v0) * size)
for age in ("young", "middleage", "old"):
    for sex in ("female", "male"):
        ffmpeg(f"{SRC}/tex/{age}_lightskinned_{sex}_diffuse.png", f"{OUT}/skin_{age}_{sex}.jpg", f"crop={w}:{h}:{x}:{y}")
for eye in ("brown", "green", "blue", "grey"):
    # En PNG : la transparence marque la cornée, qu'on ne dessine pas.
    ffmpeg(f"{SRC}/tex/{eye}_eye.png", f"{OUT}/eye_{eye}.png", "scale=512:512")
ffmpeg(f"{SRC}/tex/teeth.png", f"{OUT}/teeth.jpg", "scale=512:512")
for name in ("eyebrow001", "eyelashes01"):
    subprocess.run(["cp", f"{SRC}/tex/{name}.png", f"{OUT}/{name}.png"], check=True)
