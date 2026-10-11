"""Render optional OBJ captures from the supplied-model retargeting test.

Usage: python scripts/preview-skeleton-validation.py target/skeleton-validation
Produces a mesh contact sheet and a 32-frame diagnostic GIF, using matplotlib.
These are CPU pose diagnostics; they do not exercise the live WGPU renderer.
"""
from pathlib import Path
import sys
import numpy as np
import matplotlib
matplotlib.use("Agg")
from matplotlib import pyplot as plt
from matplotlib.collections import PolyCollection
from PIL import Image

folder = Path(sys.argv[1] if len(sys.argv) > 1 else "target/skeleton-validation")
names = ["JKA Kyle · 53 bones", "JK2 Kyle · 72 bones", "Source Juliet · 72 bones"]
colors = np.array([[0.33, 0.65, 0.89], [0.40, 0.79, 0.57], [0.94, 0.64, 0.37]])

def read_obj(path):
    vertices, faces = [], []
    for line in path.read_text().splitlines():
        if line.startswith("v "): vertices.append([float(x) for x in line.split()[1:]])
        elif line.startswith("f "): faces.append([int(x) - 1 for x in line.split()[1:]])
    return np.array(vertices), np.array(faces)

def draw(ax, model, frame):
    vertices, faces = read_obj(folder / f"model{model}-frame{frame:02}.obj")
    # Ghoul2 root already rotates the pose; an oblique front view shows hands.
    angle = np.deg2rad(-14)
    right = np.array([np.cos(angle), np.sin(angle), 0])
    depth = np.array([-np.sin(angle), np.cos(angle), 0])
    projected = np.column_stack([vertices @ right, vertices[:, 2]])
    triangles = vertices[faces]
    normals = np.cross(triangles[:, 1] - triangles[:, 0], triangles[:, 2] - triangles[:, 0])
    normals /= np.maximum(np.linalg.norm(normals, axis=1, keepdims=True), 1e-8)
    light = np.array([0.4, -0.8, 0.7]); light /= np.linalg.norm(light)
    shade = 0.4 + 0.6 * np.abs(normals @ light)
    order = np.argsort(triangles.mean(axis=1) @ depth)[::-1]
    collection = PolyCollection(projected[faces][order], facecolors=colors[model]*shade[order, None], edgecolors="none")
    ax.add_collection(collection)
    ax.set_xlim(-45, 45); ax.set_ylim(-10, 85); ax.set_aspect("equal")
    ax.set_facecolor("#101a28"); ax.axis("off")
    ax.axhline(-0.5, color="#44576c", linewidth=0.6)

fig, axes = plt.subplots(3, 3, figsize=(10, 11), facecolor="#101a28")
for row, frame in enumerate([0, 8, 24]):
    for model in range(3):
        draw(axes[row, model], model, frame)
        axes[row, model].set_title(f"{names[model]}\nSynthetic frame {frame}", color="white", fontsize=10)
fig.suptitle("Native skeleton validation · identical 72-bone pose input", color="white", fontsize=15)
fig.tight_layout(rect=(0, 0, 1, 0.96))
fig.savefig(folder / "mesh-contact-sheet.png", dpi=140)
plt.close(fig)

frames = []
for frame in range(32):
    fig, axes = plt.subplots(1, 3, figsize=(9, 4), facecolor="#101a28")
    for model in range(3):
        draw(axes[model], model, frame)
        axes[model].set_title(names[model], color="white", fontsize=11)
    fig.suptitle(f"Synthetic 72-bone pose · frame {frame:02}", color="white")
    fig.tight_layout(rect=(0, 0, 1, 0.94)); fig.canvas.draw()
    frames.append(Image.fromarray(np.asarray(fig.canvas.buffer_rgba())[:, :, :3]).convert("P", palette=Image.Palette.ADAPTIVE))
    plt.close(fig)
frames[0].save(folder / "synthetic-72-pose.gif", save_all=True, append_images=frames[1:], duration=90, loop=0)
print(folder / "mesh-contact-sheet.png")
print(folder / "synthetic-72-pose.gif")
