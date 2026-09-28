"""Retail shader classification shared with the supplied map exporter."""
def _retail_shader_family(shader_name: str) -> int:
    shader = shader_name.lower()
    if shader.startswith("environment.reflective_simple"):
        return 6
    if shader.startswith("environment.reflective_trans"):
        return 13
    if shader.startswith("environment.reflective"):
        return 5
    if shader.startswith("environment.decal_tileable"):
        return 4
    if shader.startswith("environment.decal"):
        return 3
    if shader.startswith("environment.default"):
        return 1
    if shader.startswith("environmentsimple.alphatest"):
        return 7
    if shader.startswith("environmentsimple.diffuse"):
        return 8
    if shader.startswith("environmentsimple.default"):
        return 2
    if shader.startswith("tree.default"):
        return 9
    if shader.startswith("animated.tree"):
        return 10
    if shader.startswith("proxyworld."):
        return 11
    if shader.startswith("incandescent.backlituvscroll"):
        return 14
    if shader.startswith("incandescent.default"):
        return 12
    if shader.startswith("water.flowing"):
        return 30
    if shader in ("water.default", "water.alpha", "water.skatepark"):
        return 33
    if shader.startswith("ocean.default"):
        return 31
    if shader.startswith("ocean.reflection"):
        return 32
    if shader.startswith("sky."):
        return 40
    return 0

def _retail_render_flags(shader_name: str, alpha_mode: int) -> int:
    shader = shader_name.lower()
    flags = 0
    if alpha_mode == 1:
        flags |= 1
    elif alpha_mode == 2:
        flags |= 2
    if (
        shader.startswith(("tree.", "animated.tree"))
        or "alphatest" in shader
    ):
        flags |= 1 | 4
    if shader.startswith("sky."):
        flags |= 8
    if shader.startswith("environment.decal"):
        flags |= 16
    if shader.startswith("environment.decal_tileable"):
        flags |= 32
    if shader.startswith(("water.", "ocean.")):
        flags |= 64
    return flags
