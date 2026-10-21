//! Direct2D and DirectWrite. Callers pass design-pixel coordinates; the
//! canvas scale and origin live in one transform. The render target is
//! software: a hardware device keeps roughly a hundred megabytes resident
//! for a window this simple.

use std::collections::HashMap;

use windows::{
    Win32::{
        Foundation::*,
        Graphics::{
            Direct2D::{Common::*, *},
            DirectWrite::*,
            Dxgi::Common::*,
            Gdi::*,
        },
        UI::WindowsAndMessaging::GetClientRect,
    },
    core::*,
};
use windows_numerics::{Matrix3x2, Vector2};

#[derive(Clone, Copy, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const fn rgb(hex: u32) -> Self {
        Self {
            r: ((hex >> 16) & 255) as f32 / 255.0,
            g: ((hex >> 8) & 255) as f32 / 255.0,
            b: (hex & 255) as f32 / 255.0,
            a: 1.0,
        }
    }

    fn d2d(self) -> D2D1_COLOR_F {
        D2D1_COLOR_F {
            r: self.r,
            g: self.g,
            b: self.b,
            a: self.a,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy)]
pub struct Canvas {
    pub k: f32,
    pub origin: (f32, f32),
}

struct Surface {
    rt: ID2D1RenderTarget,
    hwnd: Option<ID2D1HwndRenderTarget>,
    brush: ID2D1SolidColorBrush,
    size: (u32, u32),
}

pub struct Gfx {
    d2d: ID2D1Factory,
    dw: IDWriteFactory3,
    family: HSTRING,
    round: ID2D1StrokeStyle,
    params: Option<IDWriteRenderingParams3>,
    formats: HashMap<(u32, bool, bool), IDWriteTextFormat>,
    surface: Option<Surface>,
    canvas: Canvas,
}

impl Gfx {
    pub fn new() -> Result<Self> {
        unsafe {
            let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let dw: IDWriteFactory3 = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let round = d2d.CreateStrokeStyle(
                &D2D1_STROKE_STYLE_PROPERTIES {
                    startCap: D2D1_CAP_STYLE_ROUND,
                    endCap: D2D1_CAP_STYLE_ROUND,
                    lineJoin: D2D1_LINE_JOIN_ROUND,
                    ..Default::default()
                },
                None,
            )?;
            let params = dw
                .CreateCustomRenderingParams(
                    1.8,
                    0.5,
                    0.75,
                    0.0,
                    DWRITE_PIXEL_GEOMETRY_FLAT,
                    DWRITE_RENDERING_MODE1_NATURAL_SYMMETRIC,
                    DWRITE_GRID_FIT_MODE_DISABLED,
                )
                .ok();
            let family = pick_family(&dw);
            Ok(Self {
                d2d,
                dw,
                family: HSTRING::from(family),
                round,
                params,
                formats: HashMap::new(),
                surface: None,
                canvas: Canvas {
                    k: 1.0,
                    origin: (0.0, 0.0),
                },
            })
        }
    }

    pub fn set_canvas(&mut self, canvas: Canvas) {
        self.canvas = canvas;
    }

    /// Draw one frame into the window. Recreate the target after a device loss
    /// or a size change. Returns false when the client area is empty.
    pub fn begin_hwnd(&mut self, hwnd: HWND) -> Result<bool> {
        unsafe {
            let mut rc = RECT::default();
            let _ = GetClientRect(hwnd, &mut rc);
            let size = (rc.right.max(0) as u32, rc.bottom.max(0) as u32);
            if size.0 == 0 || size.1 == 0 {
                return Ok(false);
            }
            let recreate = self
                .surface
                .as_ref()
                .is_none_or(|surface| surface.hwnd.is_none());
            if recreate {
                self.surface = None;
                let rt = self.d2d.CreateHwndRenderTarget(
                    &target_props(),
                    &D2D1_HWND_RENDER_TARGET_PROPERTIES {
                        hwnd,
                        pixelSize: D2D_SIZE_U {
                            width: size.0,
                            height: size.1,
                        },
                        ..Default::default()
                    },
                )?;
                let brush = rt.CreateSolidColorBrush(&Color::rgb(0).d2d(), None)?;
                prepare_target(&rt, self.params.as_ref());
                self.surface = Some(Surface {
                    rt: rt.cast()?,
                    hwnd: Some(rt),
                    brush,
                    size,
                });
            } else if self.surface.as_ref().is_some_and(|surface| surface.size != size) {
                let hwnd_target = self.surface.as_ref().and_then(|surface| surface.hwnd.clone());
                if let Some(hwnd_target) = hwnd_target {
                    hwnd_target.Resize(&D2D_SIZE_U {
                        width: size.0,
                        height: size.1,
                    })?;
                    if let Some(surface) = self.surface.as_mut() {
                        surface.size = size;
                    }
                }
            }
            self.begin_draw();
            Ok(true)
        }
    }

    pub fn end(&mut self) {
        let failed = match &self.surface {
            Some(surface) => unsafe { surface.rt.EndDraw(None, None).is_err() },
            None => false,
        };
        if failed {
            self.surface = None;
        }
    }

    pub fn fill(&self, x: f32, y: f32, w: f32, h: f32, radius: f32, color: Color) {
        let Some(surface) = &self.surface else {
            return;
        };
        unsafe { surface.brush.SetColor(&color.d2d()) };
        let brush = &surface.brush;
        let rect = D2D_RECT_F {
            left: x,
            top: y,
            right: x + w,
            bottom: y + h,
        };
        unsafe {
            if radius > 0.0 {
                surface.rt.FillRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect,
                        radiusX: radius,
                        radiusY: radius,
                    },
                    brush,
                );
            } else {
                surface.rt.FillRectangle(&rect, brush);
            }
        }
    }

    pub fn stroke(&self, x: f32, y: f32, w: f32, h: f32, radius: f32, width: f32, color: Color) {
        let Some(surface) = &self.surface else {
            return;
        };
        unsafe { surface.brush.SetColor(&color.d2d()) };
        let brush = &surface.brush;
        // Inset by half the stroke so the border sits inside the box.
        let inset = width * 0.5;
        let rect = D2D_RECT_F {
            left: x + inset,
            top: y + inset,
            right: x + w - inset,
            bottom: y + h - inset,
        };
        unsafe {
            surface.rt.DrawRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect,
                    radiusX: (radius - inset).max(0.0),
                    radiusY: (radius - inset).max(0.0),
                },
                brush,
                width,
                &self.round,
            );
        }
    }

    pub fn circle(&self, cx: f32, cy: f32, radius: f32, color: Color) {
        let Some(surface) = &self.surface else {
            return;
        };
        unsafe { surface.brush.SetColor(&color.d2d()) };
        let brush = &surface.brush;
        let ellipse = D2D1_ELLIPSE {
            point: Vector2 { X: cx, Y: cy },
            radiusX: radius,
            radiusY: radius,
        };
        unsafe { surface.rt.FillEllipse(&ellipse, brush) };
    }

    pub fn circle_stroke(&self, cx: f32, cy: f32, radius: f32, width: f32, color: Color) {
        let Some(surface) = &self.surface else {
            return;
        };
        unsafe { surface.brush.SetColor(&color.d2d()) };
        let brush = &surface.brush;
        let ellipse = D2D1_ELLIPSE {
            point: Vector2 { X: cx, Y: cy },
            radiusX: radius,
            radiusY: radius,
        };
        unsafe { surface.rt.DrawEllipse(&ellipse, brush, width, &self.round) };
    }

    pub fn line(&self, x0: f32, y0: f32, x1: f32, y1: f32, width: f32, color: Color) {
        let Some(surface) = &self.surface else {
            return;
        };
        unsafe { surface.brush.SetColor(&color.d2d()) };
        let brush = &surface.brush;
        unsafe {
            surface.rt.DrawLine(
                Vector2 { X: x0, Y: y0 },
                Vector2 { X: x1, Y: y1 },
                brush,
                width,
                &self.round,
            );
        }
    }

    /// Clockwise circular arc. `large` selects the sweep longer than half a turn.
    pub fn arc(
        &self,
        from: (f32, f32),
        to: (f32, f32),
        radius: f32,
        width: f32,
        color: Color,
        large: bool,
    ) {
        let Some(surface) = &self.surface else {
            return;
        };
        unsafe { surface.brush.SetColor(&color.d2d()) };
        let brush = &surface.brush;
        unsafe {
            let Ok(geo) = self.d2d.CreatePathGeometry() else {
                return;
            };
            let Ok(sink) = geo.Open() else {
                return;
            };
            sink.BeginFigure(
                Vector2 { X: from.0, Y: from.1 },
                D2D1_FIGURE_BEGIN_HOLLOW,
            );
            sink.AddArc(&D2D1_ARC_SEGMENT {
                point: Vector2 { X: to.0, Y: to.1 },
                size: D2D_SIZE_F {
                    width: radius,
                    height: radius,
                },
                rotationAngle: 0.0,
                sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                arcSize: if large {
                    D2D1_ARC_SIZE_LARGE
                } else {
                    D2D1_ARC_SIZE_SMALL
                },
            });
            sink.EndFigure(D2D1_FIGURE_END_OPEN);
            let _ = sink.Close();
            surface.rt.DrawGeometry(&geo, brush, width, &self.round);
        }
    }

    pub fn clip(&self, x: f32, y: f32, w: f32, h: f32) {
        let Some(surface) = &self.surface else {
            return;
        };
        let rect = D2D_RECT_F {
            left: x,
            top: y,
            right: x + w,
            bottom: y + h,
        };
        unsafe {
            surface
                .rt
                .PushAxisAlignedClip(&rect, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        }
    }

    pub fn unclip(&self) {
        let Some(surface) = &self.surface else {
            return;
        };
        unsafe { surface.rt.PopAxisAlignedClip() };
    }

    pub fn measure(&mut self, text: &str, size: f32, bold: bool) -> f32 {
        if text.is_empty() {
            return 0.0;
        }
        let wide: Vec<u16> = text.encode_utf16().collect();
        let Some(layout) = self.layout(&wide, size, bold, false, 100_000.0, 100.0) else {
            return 0.0;
        };
        let mut metrics = DWRITE_TEXT_METRICS::default();
        unsafe {
            let _ = layout.GetMetrics(&mut metrics);
        }
        metrics.widthIncludingTrailingWhitespace
    }

    /// Byte offset closest to `local_x` in a single line of `text`.
    pub fn caret_index(&mut self, text: &str, size: f32, bold: bool, local_x: f32) -> usize {
        let mut best = 0;
        let mut best_dist = f32::MAX;
        let mut index = 0;
        loop {
            let width = self.measure(&text[..index], size, bold);
            let dist = (width - local_x).abs();
            if dist < best_dist {
                best_dist = dist;
                best = index;
            }
            if index >= text.len() {
                break;
            }
            index += text[index..].chars().next().map(|ch| ch.len_utf8()).unwrap_or(1);
        }
        best
    }

    pub fn text(
        &mut self,
        text: &str,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        size: f32,
        bold: bool,
        color: Color,
        align: Align,
        ellipsis: bool,
    ) {
        if text.is_empty() || w <= 0.0 || h <= 0.0 {
            return;
        }
        let wide: Vec<u16> = text.encode_utf16().collect();
        let Some(layout) = self.layout(&wide, size, bold, ellipsis, w, h) else {
            return;
        };
        unsafe {
            let _ = layout.SetTextAlignment(match align {
                Align::Left => DWRITE_TEXT_ALIGNMENT_LEADING,
                Align::Center => DWRITE_TEXT_ALIGNMENT_CENTER,
                Align::Right => DWRITE_TEXT_ALIGNMENT_TRAILING,
            });
            let Some(surface) = &self.surface else {
                return;
            };
            surface.brush.SetColor(&color.d2d());
            surface.rt.DrawTextLayout(
                Vector2 { X: x, Y: y },
                &layout,
                &surface.brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }
    }

    /// Render the design canvas into a straight RGBA buffer. Used by `--screenshot`.
    pub fn render_rgba(
        &mut self,
        width: u32,
        height: u32,
        draw: impl FnOnce(&mut Self),
    ) -> Result<Vec<u8>> {
        unsafe {
            let screen = GetDC(None);
            let memory = CreateCompatibleDC(if screen.is_invalid() { None } else { Some(screen) });
            if !screen.is_invalid() {
                let _ = ReleaseDC(None, screen);
            }
            if memory.is_invalid() {
                return Err(Error::from_thread());
            }
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width as i32,
                    biHeight: -(height as i32),
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
            let dib = CreateDIBSection(
                Some(memory),
                &info,
                DIB_RGB_COLORS,
                &mut bits,
                None,
                0,
            )?;
            let previous = SelectObject(memory, dib.into());
            let props = target_props();
            let dc = self.d2d.CreateDCRenderTarget(&props)?;
            dc.BindDC(
                memory,
                &RECT {
                    left: 0,
                    top: 0,
                    right: width as i32,
                    bottom: height as i32,
                },
            )?;
            let rt: ID2D1RenderTarget = dc.cast()?;
            let brush = rt.CreateSolidColorBrush(&Color::rgb(0).d2d(), None)?;
            prepare_target(&rt, self.params.as_ref());
            self.surface = Some(Surface {
                rt,
                hwnd: None,
                brush,
                size: (width, height),
            });
            self.begin_draw();
            draw(self);
            let failed = self
                .surface
                .as_ref()
                .unwrap()
                .rt
                .EndDraw(None, None)
                .is_err();
            self.surface = None;
            let byte_count = (width as usize) * (height as usize) * 4;
            let mut rgba = vec![0u8; byte_count];
            if !bits.is_null() && !failed {
                let src = std::slice::from_raw_parts(bits as *const u8, byte_count);
                for (pixel, bgra) in rgba.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
                    let (b, g, r, a) = (bgra[0], bgra[1], bgra[2], bgra[3]);
                    if a == 0 {
                        pixel.copy_from_slice(&[0, 0, 0, 0]);
                    } else if a == 255 {
                        pixel.copy_from_slice(&[r, g, b, 255]);
                    } else {
                        pixel[0] = ((r as u16 * 255) / a as u16) as u8;
                        pixel[1] = ((g as u16 * 255) / a as u16) as u8;
                        pixel[2] = ((b as u16 * 255) / a as u16) as u8;
                        pixel[3] = a;
                    }
                }
            }
            SelectObject(memory, previous);
            let _ = DeleteObject(dib.into());
            let _ = DeleteDC(memory);
            if failed {
                return Err(Error::from_thread());
            }
            Ok(rgba)
        }
    }

    fn begin_draw(&mut self) {
        let Some(surface) = &self.surface else {
            return;
        };
        unsafe {
            surface.rt.BeginDraw();
            surface.rt.Clear(Some(&Color::rgb(0xFFFFFF).d2d()));
            let k = self.canvas.k;
            let (ox, oy) = self.canvas.origin;
            surface.rt.SetTransform(&Matrix3x2 {
                M11: k,
                M12: 0.0,
                M21: 0.0,
                M22: k,
                M31: ox,
                M32: oy,
            });
        }
    }

    fn layout(
        &mut self,
        text: &[u16],
        size: f32,
        bold: bool,
        ellipsis: bool,
        w: f32,
        h: f32,
    ) -> Option<IDWriteTextLayout> {
        let format = self.format(size, bold, ellipsis)?;
        unsafe { self.dw.CreateTextLayout(text, &format, w.max(0.0), h.max(0.0)).ok() }
    }

    fn format(&mut self, size: f32, bold: bool, ellipsis: bool) -> Option<IDWriteTextFormat> {
        let key = ((size * 64.0).round() as u32, bold, ellipsis);
        if let Some(format) = self.formats.get(&key) {
            return Some(format.clone());
        }
        unsafe {
            let format = self
                .dw
                .CreateTextFormat(
                    &self.family,
                    None,
                    if bold {
                        DWRITE_FONT_WEIGHT_BOLD
                    } else {
                        DWRITE_FONT_WEIGHT_REGULAR
                    },
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    size,
                    w!("zh-CN"),
                )
                .ok()?;
            let _ = format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);
            let _ = format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
            if ellipsis {
                if let Ok(sign) = self.dw.CreateEllipsisTrimmingSign(&format) {
                    let trim = DWRITE_TRIMMING {
                        granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                        ..Default::default()
                    };
                    let _ = format.SetTrimming(&trim, &sign);
                }
            }
            self.formats.insert(key, format.clone());
            Some(format)
        }
    }
}

fn target_props() -> D2D1_RENDER_TARGET_PROPERTIES {
    D2D1_RENDER_TARGET_PROPERTIES {
        // Software rasterizer: this scene is arcs and text, and a hardware
        // device adds about a hundred megabytes of resident memory.
        r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
        },
        dpiX: 96.0,
        dpiY: 96.0,
        ..Default::default()
    }
}

unsafe fn prepare_target(rt: &ID2D1RenderTarget, params: Option<&IDWriteRenderingParams3>) {
    unsafe {
        rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        if let Some(params) = params {
            rt.SetTextRenderingParams(params);
        }
    }
}

/// System family, in order. DirectWrite maps the installed face; the file is
/// not copied into the process. Task names can use any character in that font.
fn pick_family(dw: &IDWriteFactory3) -> String {
    const CANDIDATES: [&str; 5] = ["等线", "DengXian", "Microsoft YaHei UI", "微软雅黑", "Segoe UI"];
    unsafe {
        let Ok(factory) = dw.cast::<IDWriteFactory>() else {
            return "Segoe UI".to_owned();
        };
        let mut collection = None;
        if factory.GetSystemFontCollection(&mut collection, false).is_err() {
            return "Segoe UI".to_owned();
        }
        let Some(collection) = collection else {
            return "Segoe UI".to_owned();
        };
        for name in CANDIDATES {
            let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
            let mut index = 0u32;
            let mut exists = BOOL(0);
            if collection
                .FindFamilyName(PCWSTR(wide.as_ptr()), &mut index, &mut exists)
                .is_ok()
                && exists.as_bool()
            {
                return name.to_owned();
            }
        }
    }
    "Segoe UI".to_owned()
}
