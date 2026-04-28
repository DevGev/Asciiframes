use image::{GenericImageView, Pixel, RgbaImage};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, ImageData};
use std::cell::RefCell;
use std::rc::Rc;

const CHARSET: &[char] = &[
    ' ', '.', '\'', '`', '^', '"', ',', ':', ';', 'I', 'l', '!', 'i', '>',
    '<', '~', '+', '_', '-', '?', ']', '[', '}', '{', '1', ')', '(', '|',
    '\\', '/', 't', 'f', 'j', 'r', 'x', 'n', 'u', 'v', 'c', 'z', 'X', 'Y',
    'U', 'J', 'C', 'L', 'Q', '0', 'O', 'Z', 'm', 'w', 'q', 'p', 'd', 'b',
    'k', 'h', 'a', 'o', '*', '#', 'M', 'W', '&', '8', '%', 'B', '@', '$',
];

#[wasm_bindgen(getter_with_clone)]
#[derive(PartialEq, Debug, Clone)]
pub struct Config {
    image_bytes: Vec<u8>,
    pub frame_count: u32,
    pub interpolation_limit: u32,
    pub frame_delay_ms: f64,
    pub pixel_fraction: f64,
    pub pixel_fraction_end: Option<f64>,
    pub enable_color: bool,
    pub background_color: String
}

#[wasm_bindgen]
impl Config {
    #[wasm_bindgen(constructor)]
    pub fn new(
        image_bytes: Vec<u8>,
        frame_count: u32,
        interpolation_limit: u32,
        frame_delay_ms: f64,
        pixel_fraction: f64,
        pixel_fraction_end: Option<f64>,
        enable_color: bool,
        background_color: String
    ) -> Self {
        Self {
            image_bytes: image_bytes,
            frame_count: frame_count,
            interpolation_limit: interpolation_limit,
            frame_delay_ms: frame_delay_ms,
            pixel_fraction: pixel_fraction.clamp(0.0, 1.0),
            pixel_fraction_end: pixel_fraction_end,
            enable_color: enable_color,
            background_color: background_color
        }
    }

    pub fn get_image_bytes(&self) -> Vec<u8> {
        self.image_bytes.clone()
    }
}

#[wasm_bindgen]
pub fn start(container_id: &str, cfg: Config) {
    console_error_panic_hook::set_once();

    let window = web_sys::window().unwrap();
    let document = window.document().unwrap();

    let container = document
        .get_element_by_id(container_id)
        .unwrap_or_else(|| {
            let el = document.create_element("div").unwrap();
            el.set_id(container_id);
            document.body().unwrap().append_child(&el).unwrap();
            el
        });

    let canvas: HtmlCanvasElement = document
        .create_element("canvas")
        .unwrap()
        .unchecked_into();

    canvas.set_attribute("style", "display:block;").unwrap();
    container.append_child(&canvas).unwrap();

    let dyn_img = image::load_from_memory(&cfg.image_bytes).expect("invalid image");
    let (img_w, img_h) = dyn_img.dimensions();
    canvas.set_width(img_w);
    canvas.set_height(img_h);

    let ctx: CanvasRenderingContext2d = canvas
        .get_context("2d")
        .unwrap()
        .unwrap()
        .unchecked_into();
    ctx.set_image_smoothing_enabled(false);

    let rgba_img = dyn_img.to_rgba8();

    let frames = Rc::new(build_frames(
        &rgba_img,
        img_w,
        img_h,
        cfg.interpolation_limit,
        cfg.frame_count as usize,
        cfg.pixel_fraction,
        cfg.pixel_fraction_end,
        cfg.enable_color,
    ));

    let frame_idx = Rc::new(RefCell::new(0usize));
    let last_time = Rc::new(RefCell::new(0.0f64));
    let frame_delay = cfg.frame_delay_ms.max(16.0);

    let raf: Rc<RefCell<Option<Closure<dyn FnMut(f64)>>>> = Rc::new(RefCell::new(None));
    let raf2 = raf.clone();

    *raf.borrow_mut() = Some(Closure::wrap(Box::new(move |now: f64| {
        if now - *last_time.borrow() >= frame_delay {
            let i = *frame_idx.borrow();
            if i < frames.len() {
                render_frame(&frames[i], &ctx, &cfg, img_w, img_h);
                *frame_idx.borrow_mut() = i + 1;
            } else {
                raf2.borrow_mut().take();
                return;
            }
            *last_time.borrow_mut() = now;
        }
        let _ = web_sys::window()
            .unwrap()
            .request_animation_frame(raf2.borrow().as_ref().unwrap().as_ref().unchecked_ref());
    }) as Box<dyn FnMut(f64)>));

    let _ = web_sys::window()
        .unwrap()
        .request_animation_frame(raf.borrow().as_ref().unwrap().as_ref().unchecked_ref());
}

#[derive(Clone)]
struct AsciiCell {
    cx: f64,
    cy: f64,
    ch: char,
    r: u8,
    g: u8,
    b: u8,
}

struct CompositeFrame {
    cells: Vec<AsciiCell>,
    font_px: f64,
    ascii_x: f64,
    ascii_w: f64,
    pixel_rgba: Vec<u8>,
    pixel_x_end: f64,
}

fn build_frames(
    src: &RgbaImage,
    img_w: u32,
    img_h: u32,
    interpolation_limit: u32,
    frame_count: usize,
    pixel_fraction: f64,
    pixel_fraction_end: Option<f64>,
    enable_color: bool,
) -> Vec<CompositeFrame> {
    let mut frames = Vec::with_capacity(frame_count);
    let split_x_start = (img_w as f64 * pixel_fraction).round();
    let split_x_end = pixel_fraction_end.map_or(split_x_start, |v| (img_w as f64 * v).round());

    let max_block = ((img_w / 4).min(img_h / 3)).max(2);

    for step in 0..frame_count {
        let t = step as f32 / (frame_count.saturating_sub(1).max(1)) as f32;

        let split_x = lerp_f64(split_x_start, split_x_end, t as f64);

        let block_ascii = lerp_u32(max_block, interpolation_limit, t).max(interpolation_limit);

        let pixel_rgba = build_pixel_rgba(src, img_w, img_h, block_ascii, enable_color);

        let cells = build_ascii_cells(
            src, img_w, img_h, block_ascii, split_x, enable_color,
        );

        let font_px = calculate_font_size(img_h, block_ascii);

        frames.push(CompositeFrame {
            cells,
            font_px,
            ascii_x: split_x,
            ascii_w: img_w as f64 - split_x,
            pixel_rgba,
            pixel_x_end: split_x,
        });
    }

    frames
}

fn calculate_font_size(
    img_h: u32,
    block_size: u32
) -> f64 {
    let rows = (img_h / block_size).clamp(1, img_h);
    let cell_h = img_h as f64 / rows as f64;
    cell_h.max(1.0)
}

fn build_ascii_cells(
    src: &RgbaImage,
    img_w: u32,
    img_h: u32,
    block_size: u32,
    split_x: f64,
    enable_color: bool,
) -> Vec<AsciiCell> {
    let cols = (img_w / block_size).clamp(1, img_w);
    let rows = (img_h / block_size).clamp(1, img_h);
    let cell_w = img_w as f64 / cols as f64;
    let cell_h = img_h as f64 / rows as f64;

    let charset_len = CHARSET.len();
    let mut cells = Vec::new();

    for row in 0..rows {
        for col in 0..cols {
            let cx = col as f64 * cell_w;
            if cx + cell_w <= split_x {
                continue;
            }

            let cy = row as f64 * cell_h;
            let x0 = col * block_size;
            let y0 = row * block_size;

            let (luma, r, g, b) = block_avg_rgba(src, x0, y0, block_size, img_w, img_h, enable_color);
            let idx = (luma as usize * charset_len / 256).min(charset_len - 1);
            let ch = CHARSET[idx];

            cells.push(AsciiCell { cx, cy, ch, r, g, b });
        }
    }
    cells
}

fn render_frame(frame: &CompositeFrame, ctx: &CanvasRenderingContext2d, cfg: &Config, w: u32, h: u32) {
    if frame.pixel_x_end > 0.0 {
        let clamped = wasm_bindgen::Clamped(frame.pixel_rgba.as_slice());
        let image_data = ImageData::new_with_u8_clamped_array_and_sh(clamped, w, h).unwrap();

        ctx.put_image_data_with_dirty_x_and_dirty_y_and_dirty_width_and_dirty_height(
            &image_data,
            0.0,
            0.0,
            0.0,
            0.0,
            frame.pixel_x_end,
            h as f64,
        )
        .unwrap();
    }

    if frame.ascii_w > 0.0 {
        ctx.set_fill_style_str(cfg.background_color.as_str());
        ctx.fill_rect(frame.ascii_x, 0.0, frame.ascii_w, h as f64);

        ctx.set_font(&format!(
            "{}px 'Courier New', Courier, monospace",
            frame.font_px.floor().max(1.0) as u32
        ));
        ctx.set_text_baseline("top");
        ctx.set_text_align("left");

        ctx.save();
        ctx.begin_path();
        ctx.rect(frame.ascii_x, 0.0, frame.ascii_w, h as f64);
        ctx.clip();

        let mut last_color = String::new();
        let mut buf = [0u8; 4];

        for cell in &frame.cells {
            let color_str = format!("rgb({},{},{})", cell.r, cell.g, cell.b);
            if color_str != last_color {
                ctx.set_fill_style_str(&color_str);
                last_color = color_str;
            }
            let s = cell.ch.encode_utf8(&mut buf);
            let _ = ctx.fill_text(s, cell.cx, cell.cy);
        }
        ctx.restore();

        if frame.pixel_x_end > 0.0 && frame.ascii_w > 0.0 {
            ctx.set_stroke_style_str("#444444");
            ctx.set_line_width(1.0);
            ctx.begin_path();
            ctx.move_to(frame.ascii_x, 0.0);
            ctx.line_to(frame.ascii_x, h as f64);
            ctx.stroke();
        }
    }
}

fn build_pixel_rgba(
    src: &RgbaImage,
    img_w: u32,
    img_h: u32,
    block_size: u32,
    enable_color: bool,
) -> Vec<u8> {
    let mut rgba = vec![0u8; (img_w * img_h * 4) as usize];
    let bw = (img_w + block_size - 1) / block_size;
    let bh = (img_h + block_size - 1) / block_size;

    for by in 0..bh {
        for bx in 0..bw {
            let x0 = bx * block_size;
            let y0 = by * block_size;
            let (luma, r, g, b) = block_avg_rgba(src, x0, y0, block_size, img_w, img_h, enable_color);
            let (pr, pg, pb) = if enable_color { (r, g, b) } else { (luma, luma, luma) };

            let x1 = (x0 + block_size).min(img_w);
            let y1 = (y0 + block_size).min(img_h);

            for py in y0..y1 {
                for px in x0..x1 {
                    let i = ((py * img_w + px) * 4) as usize;
                    rgba[i] = pr;
                    rgba[i + 1] = pg;
                    rgba[i + 2] = pb;
                    rgba[i + 3] = 255;
                }
            }
        }
    }
    rgba
}

#[inline]
fn block_avg_rgba(
    src: &RgbaImage,
    x0: u32,
    y0: u32,
    size: u32,
    w: u32,
    h: u32,
    enable_color: bool,
) -> (u8, u8, u8, u8) {
    let x1 = (x0 + size).min(w);
    let y1 = (y0 + size).min(h);

    let mut sr = 0u64;
    let mut sg = 0u64;
    let mut sb = 0u64;
    let mut count = 0u64;

    for py in y0..y1 {
        for px in x0..x1 {
            let p = src.get_pixel(px, py).to_rgba();
            sr += p[0] as u64;
            sg += p[1] as u64;
            sb += p[2] as u64;
            count += 1;
        }
    }

    if count == 0 {
        return (0, 0, 0, 0);
    }

    let r = (sr / count) as u8;
    let g = (sg / count) as u8;
    let b = (sb / count) as u8;
    let luma = ((0.2126 * r as f32) + (0.7152 * g as f32) + (0.0722 * b as f32)) as u8;

    if enable_color {
        (luma, r, g, b)
    } else {
        (luma, luma, luma, luma)
    }
}

#[inline]
fn lerp_u32(a: u32, b: u32, t: f32) -> u32 {
    let (a, b) = (a as f32, b as f32);
    (a + (b - a) * t).round() as u32
}

#[inline]
fn lerp_f64(a: f64, b: f64, t: f64) -> f64 {
    let (a, b) = (a as f64, b as f64);
    (a + (b - a) * t).round() as f64
}
