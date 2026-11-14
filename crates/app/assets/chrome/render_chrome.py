"""Renders the chrome objects behind the glass panel as PNG sprites with alpha.

    blender -b --python crates/app/assets/chrome/render_chrome.py

Lighting: Blender's bundled studio HDRI (CC0, Poly Haven) plus three soft boxes,
one warm white, one cool white and one dim gold, so the chrome picks up warm and cool
reflections. Every object is rendered alone with an orthographic camera fitted to it.
"""
import math
import os

import bpy

OUT = os.path.dirname(os.path.abspath(__file__))
HDRI = os.path.join(os.path.dirname(bpy.app.binary_path), f"{bpy.app.version[0]}.{bpy.app.version[1]}",
                    "datafiles", "studiolights", "world", "studio.exr")

# name, kind, pixels, rig turn (degrees), object rotation (degrees)
SPRITES = [
    ("sphere_big", "sphere", 640, 0, (0, 0, 0)),
    ("sphere_mid", "sphere", 256, 70, (0, 0, 0)),
    ("sphere_small", "sphere", 176, -55, (0, 0, 0)),
    ("sphere_tiny", "sphere", 112, 150, (0, 0, 0)),
    ("torus", "torus", 560, 25, (62, 0, 28)),
]


def emission_box(name, size, location, color, strength):
    bpy.ops.mesh.primitive_plane_add(size=1, location=location)
    box = bpy.context.object
    box.name = name
    box.scale = (size[0], size[1], 1)
    direction = -box.location.normalized()
    box.rotation_euler = direction.to_track_quat("Z", "Y").to_euler()
    material = bpy.data.materials.new(name)
    material.use_nodes = True
    nodes = material.node_tree.nodes
    nodes.clear()
    emit = nodes.new("ShaderNodeEmission")
    emit.inputs["Color"].default_value = (*color, 1)
    emit.inputs["Strength"].default_value = strength
    out = nodes.new("ShaderNodeOutputMaterial")
    material.node_tree.links.new(emit.outputs[0], out.inputs[0])
    box.data.materials.append(material)
    box.visible_camera = False
    return box


def main():
    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    scene.render.engine = "CYCLES"
    scene.cycles.samples = 384
    scene.cycles.use_denoising = True
    scene.render.film_transparent = True
    scene.render.image_settings.file_format = "PNG"
    scene.render.image_settings.color_mode = "RGBA"
    scene.view_settings.view_transform = "Standard"
    scene.view_settings.exposure = -0.2

    world = bpy.data.worlds.new("studio")
    scene.world = world
    world.use_nodes = True
    nodes = world.node_tree.nodes
    env = nodes.new("ShaderNodeTexEnvironment")
    env.image = bpy.data.images.load(HDRI)
    # A photo studio's sweep for the steel look: cool bright top, warm horizon strip, dark floor.
    # The HDRI is added on top for the irregular detail of a real room.
    coords = nodes.new("ShaderNodeTexCoord")
    split = nodes.new("ShaderNodeSeparateXYZ")
    ramp = nodes.new("ShaderNodeValToRGB")
    ramp.color_ramp.interpolation = "EASE"
    stops = [(0.0, (0.02, 0.02, 0.02)), (0.46, (0.07, 0.07, 0.065)), (0.505, (1.0, 0.95, 0.86)),
             (0.56, (0.42, 0.43, 0.44)), (0.8, (0.62, 0.66, 0.72)), (1.0, (0.8, 0.84, 0.9))]
    ramp.color_ramp.elements[0].position, ramp.color_ramp.elements[0].color = stops[0][0], (*stops[0][1], 1)
    ramp.color_ramp.elements[1].position, ramp.color_ramp.elements[1].color = stops[-1][0], (*stops[-1][1], 1)
    for position, color in stops[1:-1]:
        ramp.color_ramp.elements.new(position).color = (*color, 1)
    to_unit = nodes.new("ShaderNodeMapRange")
    to_unit.inputs["From Min"].default_value = -1.0
    add = nodes.new("ShaderNodeMix")
    add.data_type = "RGBA"
    add.blend_type = "ADD"
    add.inputs["Factor"].default_value = 0.7
    links = world.node_tree.links
    links.new(coords.outputs["Generated"], split.inputs[0])
    links.new(split.outputs["Z"], to_unit.inputs["Value"])
    links.new(to_unit.outputs["Result"], ramp.inputs["Fac"])
    links.new(ramp.outputs["Color"], add.inputs["A"])
    links.new(env.outputs["Color"], add.inputs["B"])
    background = nodes["Background"]
    background.inputs["Strength"].default_value = 1.0
    links.new(add.outputs["Result"], background.inputs["Color"])

    rig = bpy.data.objects.new("rig", None)
    scene.collection.objects.link(rig)
    for box in [
        emission_box("warm", (6, 3), (-5, -4, 6), (1.0, 0.93, 0.84), 9),
        emission_box("cool", (3, 7), (7, -3, 1), (0.82, 0.9, 1.0), 6),
        emission_box("gold", (8, 1.2), (0, -5, -4), (0.95, 0.78, 0.42), 2.5),
        emission_box("olive", (2, 4), (-7, 2, -1), (0.62, 0.64, 0.38), 1.6),
    ]:
        box.parent = rig

    chrome = bpy.data.materials.new("chrome")
    chrome.use_nodes = True
    bsdf = chrome.node_tree.nodes["Principled BSDF"]
    bsdf.inputs["Base Color"].default_value = (0.86, 0.86, 0.87, 1)
    bsdf.inputs["Metallic"].default_value = 1.0
    bsdf.inputs["Roughness"].default_value = 0.06

    camera = bpy.data.objects.new("camera", bpy.data.cameras.new("camera"))
    camera.data.type = "ORTHO"
    scene.collection.objects.link(camera)
    scene.camera = camera
    camera.location = (0, -20, 0)
    camera.rotation_euler = (math.radians(90), 0, 0)

    for name, kind, pixels, turn, rotation in SPRITES:
        for obj in [o for o in scene.objects if o.get("sprite")]:
            bpy.data.objects.remove(obj)
        if kind == "sphere":
            bpy.ops.mesh.primitive_uv_sphere_add(segments=128, ring_count=64, radius=1)
            extent = 2.0
        else:
            bpy.ops.mesh.primitive_torus_add(major_radius=1, minor_radius=0.3, major_segments=192, minor_segments=64)
            extent = 2.6
        obj = bpy.context.object
        obj["sprite"] = True
        obj.rotation_euler = [math.radians(a) for a in rotation]
        obj.data.materials.append(chrome)
        bpy.ops.object.shade_smooth()
        rig.rotation_euler = (0, 0, math.radians(turn))
        camera.data.ortho_scale = extent * 1.02
        scene.render.resolution_x = scene.render.resolution_y = pixels
        scene.render.filepath = os.path.join(OUT, f"{name}.png")
        bpy.ops.render.render(write_still=True)


main()
