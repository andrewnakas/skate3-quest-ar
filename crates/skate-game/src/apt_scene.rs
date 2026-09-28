//! Retained movie traversal. All geometry and glyphs come from the owned APT.
use crate::{apt_movie::Movie, apt_vm::Vm};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Clone, Deserialize)]
pub struct Vertex {
    pub position: [f32; 2],
    pub uv: [f32; 2],
}
#[derive(Clone, Deserialize)]
pub struct Texture {
    pub width: u32,
    pub height: u32,
    pub rgba: String,
}
#[derive(Clone, Deserialize)]
pub struct Shape {
    pub texture: Texture,
    pub triangles: Vec<[Vertex; 3]>,
    pub color: [f32; 4],
}
pub type Shapes = BTreeMap<i32, Vec<Shape>>;
pub struct Draw {
    pub texture: String,
    pub size: [u32; 2],
    pub vertices: Vec<Vertex>,
    pub multiply: [f32; 4],
    pub add: [f32; 4],
}
fn compose(a: [f32; 6], b: [f32; 6]) -> [f32; 6] {
    [
        a[0] * b[0] + a[2] * b[1],
        a[1] * b[0] + a[3] * b[1],
        a[0] * b[2] + a[2] * b[3],
        a[1] * b[2] + a[3] * b[3],
        a[0] * b[4] + a[2] * b[5] + a[4],
        a[1] * b[4] + a[3] * b[5] + a[5],
    ]
}
fn transform(m: [f32; 6], v: &Vertex) -> Vertex {
    Vertex {
        position: [
            m[0] * v.position[0] + m[2] * v.position[1] + m[4],
            m[1] * v.position[0] + m[3] * v.position[1] + m[5],
        ],
        uv: v.uv,
    }
}
pub fn draw(movie: &Movie, vm: &Vm, shapes: &Shapes) -> Result<Vec<Draw>, String> {
    let mut draws = Vec::new();
    visit(
        movie,
        vm,
        shapes,
        movie.root,
        [1., 0., 0., 1., 0., 0.],
        [1.; 4],
        [0.; 4],
        &mut draws,
    )?;
    Ok(draws)
}
fn visit(
    movie: &Movie,
    vm: &Vm,
    shapes: &Shapes,
    id: usize,
    parent: [f32; 6],
    pm: [f32; 4],
    pa: [f32; 4],
    out: &mut Vec<Draw>,
) -> Result<(), String> {
    if !vm.get(id, "_visible").truth() {
        return Ok(());
    }
    let instance = &movie.instances[&id];
    let mut matrix = instance
        .placement
        .as_ref()
        .map_or([1., 0., 0., 1., 0., 0.], |p| p.matrix);
    matrix[4] = vm.get(id, "_x").number() as f32;
    matrix[5] = vm.get(id, "_y").number() as f32;
    let matrix = compose(parent, matrix);
    if matrix.iter().any(|x| !x.is_finite()) {
        return Err(format!(
            "Nonfinite HUD transform on character {}",
            instance.character
        ));
    }
    let color = instance
        .placement
        .as_ref()
        .map_or([255, 255, 255, 255, 0, 0, 0, 0], |p| p.color);
    let rgba = [1, 2, 3, 0];
    let mut multiply = std::array::from_fn(|i| pm[i] * color[rgba[i]] as f32 / 255.);
    let mut add = std::array::from_fn(|i| pa[i] + pm[i] * color[4 + rgba[i]] as f32 / 255.);
    let alpha = (vm.get(id, "_alpha").number() as f32 / 100.).clamp(0., 1.);
    multiply[3] *= alpha;
    add[3] *= alpha;
    if multiply[3] <= 0. && add[3] <= 0. {
        return Ok(());
    }
    let character = &movie.characters[&instance.character];
    if character.type_name == "shape" {
        for shape in shapes
            .get(&character.id)
            .ok_or("Missing original shape geometry")?
        {
            out.push(Draw {
                texture: shape.texture.rgba.clone(),
                size: [shape.texture.width, shape.texture.height],
                vertices: shape
                    .triangles
                    .iter()
                    .flatten()
                    .map(|v| transform(matrix, v))
                    .collect(),
                multiply: std::array::from_fn(|i| multiply[i] * shape.color[i]),
                add,
            });
        }
    } else if let Some(text) = &character.text {
        let font = &movie.text_assets.fonts
            [&(text["font_id"].as_i64().ok_or("Invalid text font")? as i32)];
        // Native shadow text uses a black atlas pass followed by the sharp
        // futuraheavy glyphs translated +1 in text X (825D6B68/82CA1FD8).
        let passes = if let Some(foreground) = font.foreground {
            vec![
                (font, true, 0.),
                (&movie.text_assets.fonts[&foreground], false, 1.),
            ]
        } else {
            vec![(font, false, 0.)]
        };
        for (font, shadow, advance_x) in passes {
            let height = text["font_height"].as_f64().ok_or("Invalid text height")? as f32;
            let sx = font.scale[0] * height;
            let sy = font.scale[1] * height;
            let value = vm.get(id, "_displayText").text();
            let bounds = character.bounds.ok_or("Missing text bounds")?;
            let width = font.width(&value, height);
            let alignment = text["alignment"].as_u64().unwrap_or(0);
            let autosize = vm.get(id, "autoSize").text();
            let alignment = if autosize == "left" { 0 } else { alignment };
            let mut x = bounds[0]
                + advance_x
                + font.offset[0] * sx
                + match alignment {
                    1 => bounds[2] - bounds[0] - width,
                    2 => (bounds[2] - bounds[0] - width) * 0.5,
                    _ => 0.,
                };
            let y = bounds[1] + font.offset[1] * sy;
            let mut vertices = Vec::new();
            for c in value.chars() {
                if let Some(g) = font.glyph(c) {
                    let x0 = x + g.x_offset * sx;
                    let y0 = y + (font.ascent - g.y_offset) * sy;
                    let x1 = x0 + g.width * sx;
                    let y1 = y0 + g.height * sy;
                    let [u0, v0, u1, v1] = g.atlas_bounds;
                    let points = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
                    let uvs = [[u0, v0], [u1, v0], [u1, v1], [u0, v1]];
                    for i in [0, 1, 2, 0, 2, 3] {
                        vertices.push(transform(
                            matrix,
                            &Vertex {
                                position: points[i],
                                uv: [
                                    uvs[i][0] / font.size[0] as f32,
                                    uvs[i][1] / font.size[1] as f32,
                                ],
                            },
                        ));
                    }
                    x = g.x_advance.mul_add(sx, x);
                }
            }
            let argb = u32::from_str_radix(
                text["color_argb"]
                    .as_str()
                    .ok_or("Missing text color")?
                    .trim_start_matches('#'),
                16,
            )
            .map_err(|e| e.to_string())?;
            let color = [
                ((argb >> 16) & 255) as f32 / 255.,
                ((argb >> 8) & 255) as f32 / 255.,
                (argb & 255) as f32 / 255.,
                (argb >> 24) as f32 / 255.,
            ];
            if !vertices.is_empty() {
                out.push(Draw {
                    texture: font.texture.clone(),
                    size: font.size,
                    vertices,
                    multiply: std::array::from_fn(|i| {
                        if shadow && i < 3 {
                            0.
                        } else {
                            multiply[i] * color[i]
                        }
                    }),
                    add,
                });
            }
        }
    }
    for child in instance.children.values() {
        visit(movie, vm, shapes, *child, matrix, multiply, add, out)?;
    }
    Ok(())
}
