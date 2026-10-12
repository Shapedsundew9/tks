"""World (environment) helpers: HDRI, physical sky, solid colour."""

import math

import bpy


def _reset_world(scene, name):
    scene = scene or bpy.context.scene
    world = scene.world or bpy.data.worlds.new(name)
    scene.world = world
    world.use_nodes = True
    nt = world.node_tree
    nt.nodes.clear()
    out = nt.nodes.new("ShaderNodeOutputWorld")
    out.location = (400, 0)
    bg = nt.nodes.new("ShaderNodeBackground")
    bg.location = (150, 0)
    nt.links.new(bg.outputs["Background"], out.inputs["Surface"])
    return world, nt, bg


def hdri(path, strength=1.0, rotation_deg=0.0, scene=None):
    """Image-based lighting from an .hdr/.exr file (e.g. a Poly Haven HDRI)."""
    world, nt, bg = _reset_world(scene, "HDRI_World")
    env = nt.nodes.new("ShaderNodeTexEnvironment")
    env.location = (-200, 0)
    env.image = bpy.data.images.load(str(path), check_existing=True)
    mapping = nt.nodes.new("ShaderNodeMapping")
    mapping.location = (-400, 0)
    coord = nt.nodes.new("ShaderNodeTexCoord")
    coord.location = (-600, 0)
    mapping.inputs["Rotation"].default_value[2] = math.radians(rotation_deg)
    nt.links.new(coord.outputs["Generated"], mapping.inputs["Vector"])
    nt.links.new(mapping.outputs["Vector"], env.inputs["Vector"])
    nt.links.new(env.outputs["Color"], bg.inputs["Color"])
    bg.inputs["Strength"].default_value = strength
    return world


def sky(sun_elevation_deg=15.0, sun_rotation_deg=0.0, strength=1.0, sky_type="MULTIPLE_SCATTERING",
        altitude=0.0, air_density=1.0, aerosol_density=1.0, ozone_density=1.0, sun_intensity=1.0,
        scene=None):
    """Physical sky. Blender 5.x types: MULTIPLE_SCATTERING (default), SINGLE_SCATTERING,
    PREETHAM, HOSEK_WILKIE. ``NISHITA`` no longer exists."""
    world, nt, bg = _reset_world(scene, "Sky_World")
    tex = nt.nodes.new("ShaderNodeTexSky")
    tex.location = (-200, 0)
    tex.sky_type = sky_type
    if sky_type in {"MULTIPLE_SCATTERING", "SINGLE_SCATTERING"}:
        tex.sun_elevation = math.radians(sun_elevation_deg)
        tex.sun_rotation = math.radians(sun_rotation_deg)
        tex.altitude = altitude
        tex.air_density = air_density
        tex.aerosol_density = aerosol_density
        tex.ozone_density = ozone_density
        tex.sun_intensity = sun_intensity
    nt.links.new(tex.outputs["Color"], bg.inputs["Color"])
    bg.inputs["Strength"].default_value = strength
    return world


def solid(color=(0.05, 0.05, 0.05), strength=1.0, scene=None):
    """Uniform background colour (studio / product shots)."""
    world, nt, bg = _reset_world(scene, "Solid_World")
    bg.inputs["Color"].default_value = (*color[:3], 1.0)
    bg.inputs["Strength"].default_value = strength
    return world
