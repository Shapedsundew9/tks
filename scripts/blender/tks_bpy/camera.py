"""Camera rig helpers."""

import bpy
from mathutils import Vector

from . import scene as _scene

FOCAL_PRESETS = {"wide": 24, "standard": 35, "normal": 50, "portrait": 85, "tele": 135, "product": 100}


def rig(location=(7, -7, 4), target=(0, 0, 1), lens=50, name="Camera", dof=False, fstop=2.8,
        sensor_width=36.0, collection="Cameras", make_active=True):
    """Camera aimed at a focus empty (Track To). ``lens`` may be mm or a FOCAL_PRESETS key.

    Returns (camera_obj, target_empty). Move the empty to re-aim / refocus.
    """
    if isinstance(lens, str):
        lens = FOCAL_PRESETS[lens]
    data = bpy.data.cameras.new(name)
    data.lens = lens
    data.sensor_width = sensor_width
    data.clip_start = 0.05
    data.clip_end = 2000
    cam = bpy.data.objects.new(name, data)
    cam.location = location
    _scene.link(cam, collection)
    empty = bpy.data.objects.new(f"{name}_Focus", None)
    empty.empty_display_type = "SPHERE"
    empty.empty_display_size = 0.1
    empty.location = Vector(target)
    _scene.link(empty, collection)
    con = cam.constraints.new("TRACK_TO")
    con.target = empty
    con.track_axis = "TRACK_NEGATIVE_Z"
    con.up_axis = "UP_Y"
    data.dof.use_dof = dof
    data.dof.focus_object = empty
    data.dof.aperture_fstop = fstop
    if make_active:
        bpy.context.scene.camera = cam
    return cam, empty


def frame_objects(cam, objs, margin=1.15):
    """Dolly ``cam`` along its view axis so ``objs`` fit the frame (keeps direction)."""
    import math

    bpy.context.view_layer.update()
    lo, hi = _scene.world_bbox(objs)
    centre = (lo + hi) / 2
    radius = (hi - lo).length / 2 * margin
    sc = bpy.context.scene
    aspect = sc.render.resolution_x / sc.render.resolution_y
    # Sensor fit AUTO: lens angle spans the longer side, so the shorter side limits framing.
    fov = 2 * math.atan(math.tan(cam.data.angle / 2) / max(aspect, 1 / aspect))
    dist = radius / math.sin(fov / 2)
    direction = (cam.matrix_world.translation - centre).normalized()
    cam.location = centre + direction * dist
    for con in cam.constraints:
        if con.type == "TRACK_TO" and con.target is not None:
            con.target.location = centre
    return cam
