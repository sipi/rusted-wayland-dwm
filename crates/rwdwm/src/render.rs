//! Composition d'une sortie.
//!
//! [`output_elements`] construit la liste des éléments de rendu **de l'avant
//! vers l'arrière**, dans l'ordre d'empilement de dwm étendu aux couches
//! *layer-shell* :
//!
//! ```text
//! curseur · overlay · plein écran · top · popups · flottants · barre · tuilés · bottom · background
//! ```
//!
//! Le code est générique sur le moteur de rendu : il sert tel quel à GLES
//! (backends `udev` et `winit`) et à Pixman (backend `headless`).

use std::time::Duration;

use rwdwm_core::bar;
use rwdwm_core::config::Color;
use rwdwm_core::model::{ClientId, MonitorId};
use rwdwm_core::Rect;
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement};
use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::surface::{render_elements_from_surface_tree, WaylandSurfaceRenderElement};
use smithay::backend::renderer::element::utils::CropRenderElement;
use smithay::backend::renderer::element::{render_elements, AsRenderElements, Kind};
use smithay::backend::renderer::{Color32F, ImportAll, ImportMem, Renderer};
use smithay::desktop::utils::send_frames_surface_tree;
use smithay::desktop::{layer_map_for_output, PopupManager};
use smithay::input::pointer::{CursorIcon, CursorImageStatus, CursorImageSurfaceData};
use smithay::output::Output;
use smithay::utils::{Logical, Physical, Point, Rectangle, Scale, Transform};
use smithay::wayland::compositor::with_states;
use smithay::wayland::shell::wlr_layer::Layer;
use tracing::warn;

use crate::state::{BarCache, CursorMode, State};

render_elements! {
    /// Élément de rendu d'une sortie.
    pub OutputElement<R> where R: ImportAll + ImportMem;
    /// Surface Wayland (couches, popups, curseur).
    Surface=WaylandSurfaceRenderElement<R>,
    /// Surface d'un client, rognée à la zone qui lui est allouée.
    Cropped=CropRenderElement<WaylandSurfaceRenderElement<R>>,
    /// Aplat (bordures, fond du plein écran).
    Solid=SolidColorRenderElement,
    /// Image en mémoire (barre, curseur du thème).
    Memory=MemoryRenderBufferRenderElement<R>,
}

/// Couleur du noyau → couleur Smithay.
pub fn color32(color: Color) -> Color32F {
    Color32F::from(color.rgba_f32())
}

/// Rectangle logique global → rectangle physique relatif à la sortie.
fn to_physical(rect: Rect, origin: Point<i32, Logical>, scale: Scale<f64>) -> Rectangle<i32, Physical> {
    let loc = Point::<i32, Logical>::from((rect.x - origin.x, rect.y - origin.y));
    let size = smithay::utils::Size::<i32, Logical>::from((rect.w.max(0), rect.h.max(0)));
    Rectangle::new(loc.to_physical_precise_round(scale), size.to_physical_precise_round(scale))
}

/// Construit les éléments de rendu d'une sortie, de l'avant vers l'arrière.
pub fn output_elements<R>(state: &mut State, renderer: &mut R, output: &Output) -> Vec<OutputElement<R>>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Clone + Send + 'static,
{
    let mut out = Vec::new();
    let Some(mon) = state.outputs.iter().find(|e| &e.output == output).map(|e| e.mon) else { return out };
    let Some(geo) = state.space.output_geometry(output) else { return out };
    let scale_int = output.current_scale().integer_scale();
    let scale = Scale::from(output.current_scale().fractional_scale());
    let origin = geo.loc;

    cursor_elements(state, renderer, origin, geo, scale_int, scale, &mut out);
    layer_elements(renderer, output, Layer::Overlay, scale, &mut out);

    let stacking = state.wm.stacking(mon);
    if let Some(&id) = stacking.fullscreen.last() {
        // Un client en plein écran masque tout : fond noir derrière lui.
        popup_elements(state, renderer, id, origin, scale, &mut out);
        client_elements(state, renderer, id, origin, scale, &mut out);
        if let Some(entry) = state.outputs.iter_mut().find(|e| e.mon == mon) {
            entry.fullscreen_backdrop.update(geo.size, Color32F::new(0.0, 0.0, 0.0, 1.0));
            out.push(OutputElement::Solid(SolidColorRenderElement::from_buffer(
                &entry.fullscreen_backdrop,
                Point::<i32, Physical>::from((0, 0)),
                scale,
                1.0,
                Kind::Unspecified,
            )));
        }
        return out;
    }

    layer_elements(renderer, output, Layer::Top, scale, &mut out);

    let screen = Rect::new(geo.loc.x, geo.loc.y, geo.size.w, geo.size.h);
    let on_screen =
        |state: &State, id: &ClientId| state.wm.client(*id).is_some_and(|c| c.geom.intersection_area(&screen) > 0);
    let floating: Vec<ClientId> =
        state.floating_front_to_back(mon).into_iter().filter(|id| on_screen(state, id)).collect();
    let tiled: Vec<ClientId> = stacking.tiled.iter().rev().copied().collect();

    for &id in floating.iter().chain(&tiled) {
        popup_elements(state, renderer, id, origin, scale, &mut out);
    }
    for &id in &floating {
        client_elements(state, renderer, id, origin, scale, &mut out);
    }
    bar_element(state, renderer, mon, origin, scale_int, scale, &mut out);
    for &id in &tiled {
        client_elements(state, renderer, id, origin, scale, &mut out);
    }
    layer_elements(renderer, output, Layer::Bottom, scale, &mut out);
    layer_elements(renderer, output, Layer::Background, scale, &mut out);
    out
}

/// Surfaces *layer-shell* d'une couche (popups compris).
fn layer_elements<R>(
    renderer: &mut R,
    output: &Output,
    layer: Layer,
    scale: Scale<f64>,
    out: &mut Vec<OutputElement<R>>,
) where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Clone + Send + 'static,
{
    let map = layer_map_for_output(output);
    for surface in map.layers_on(layer).rev() {
        let Some(geo) = map.layer_geometry(surface) else { continue };
        let loc = geo.loc.to_physical_precise_round(scale);
        out.extend(surface.render_elements::<OutputElement<R>>(renderer, loc, scale, 1.0));
    }
}

/// Popups d'un client (non rognés : un menu peut déborder de sa fenêtre).
fn popup_elements<R>(
    state: &State,
    renderer: &mut R,
    id: ClientId,
    origin: Point<i32, Logical>,
    scale: Scale<f64>,
    out: &mut Vec<OutputElement<R>>,
) where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Clone + Send + 'static,
{
    let (Some(client), Some(managed)) = (state.wm.client(id), state.windows.get(&id)) else { return };
    let Some(toplevel) = managed.window.toplevel().filter(|_| managed.mapped) else { return };
    let inner = client.inner();
    let base = Point::<i32, Logical>::from((inner.x, inner.y)) - origin;
    for (popup, location) in PopupManager::popups_for_surface(toplevel.wl_surface()) {
        let loc = (base + location - popup.geometry().loc).to_physical_precise_round(scale);
        out.extend(render_elements_from_surface_tree::<R, OutputElement<R>>(
            renderer,
            popup.wl_surface(),
            loc,
            scale,
            1.0,
            Kind::Unspecified,
        ));
    }
}

/// Un client : sa surface, rognée à la zone allouée, puis ses bordures.
fn client_elements<R>(
    state: &mut State,
    renderer: &mut R,
    id: ClientId,
    origin: Point<i32, Logical>,
    scale: Scale<f64>,
    out: &mut Vec<OutputElement<R>>,
) where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Clone + Send + 'static,
{
    let Some(client) = state.wm.client(id) else { return };
    let Some(managed) = state.windows.get_mut(&id).filter(|m| m.mapped) else { return };
    let Some(toplevel) = managed.window.toplevel() else { return };
    let inner = client.inner();

    // Contenu. La géométrie de la fenêtre peut être décalée dans la surface
    // (ombres côté client) : on place l'origine de la géométrie en `inner`.
    let window_geo = managed.window.geometry();
    let loc =
        (Point::<i32, Logical>::from((inner.x, inner.y)) - window_geo.loc - origin).to_physical_precise_round(scale);
    let crop = to_physical(inner, origin, scale);
    let surfaces: Vec<WaylandSurfaceRenderElement<R>> =
        render_elements_from_surface_tree(renderer, toplevel.wl_surface(), loc, scale, 1.0, Kind::Unspecified);
    out.extend(
        surfaces
            .into_iter()
            .filter_map(|e| CropRenderElement::from_element(e, scale, crop))
            .map(OutputElement::Cropped),
    );

    // Bordures, tracées autour de la zone allouée.
    let bw = client.bw;
    if bw <= 0 {
        return;
    }
    let colors = &state.wm.config().colors;
    let is_selected = state.wm.selected() == Some(id);
    let color = if client.is_urgent {
        colors.urgent_border
    } else if is_selected {
        colors.sel.border
    } else {
        colors.norm.border
    };
    let g = client.geom;
    let rects = [
        Rect::new(g.x, g.y, g.w, bw),
        Rect::new(g.x, g.bottom() - bw, g.w, bw),
        Rect::new(g.x, g.y + bw, bw, g.h - 2 * bw),
        Rect::new(g.right() - bw, g.y + bw, bw, g.h - 2 * bw),
    ];
    for (buffer, rect) in managed.borders.iter_mut().zip(rects) {
        if rect.is_empty() {
            continue;
        }
        buffer.update((rect.w, rect.h), color32(color));
        let loc = Point::<i32, Logical>::from((rect.x - origin.x, rect.y - origin.y));
        out.push(OutputElement::Solid(SolidColorRenderElement::from_buffer(
            buffer,
            loc.to_physical_precise_round::<_, i32>(scale),
            scale,
            1.0,
            Kind::Unspecified,
        )));
    }
}

/// La barre du moniteur ; l'image n'est régénérée que si son contenu change.
fn bar_element<R>(
    state: &mut State,
    renderer: &mut R,
    mon: MonitorId,
    origin: Point<i32, Logical>,
    scale_int: i32,
    scale: Scale<f64>,
    out: &mut Vec<OutputElement<R>>,
) where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Clone + Send + 'static,
{
    let (Some(rect), Some(view)) = (state.wm.bar_rect(mon), state.wm.bar_view(mon)) else { return };
    let Some(entry) = state.outputs.iter_mut().find(|e| e.mon == mon) else { return };

    let fresh = entry.bar.as_ref().is_some_and(|c| c.view == view && c.width == rect.w && c.scale == scale_int);
    if !fresh {
        let pixmap = bar::draw(&view, rect.w, scale_int, &state.wm.config().colors);
        let size = (pixmap.width, pixmap.height);
        let buffer = MemoryRenderBuffer::from_slice(
            &pixmap.data,
            Fourcc::Argb8888,
            size,
            scale_int,
            Transform::Normal,
            Some(vec![Rectangle::from_size(size.into())]),
        );
        entry.bar = Some(BarCache { view, width: rect.w, scale: scale_int, buffer });
    }
    let Some(cache) = entry.bar.as_ref() else { return };
    let loc = Point::<i32, Logical>::from((rect.x - origin.x, rect.y - origin.y));
    let loc: Point<f64, Physical> = loc.to_f64().to_physical(scale);
    match MemoryRenderBufferRenderElement::from_buffer(
        renderer,
        loc,
        &cache.buffer,
        None,
        None,
        None,
        Kind::Unspecified,
    ) {
        Ok(element) => out.push(OutputElement::Memory(element)),
        Err(err) => warn!(?err, "barre non importée"),
    }
}

/// Le curseur et l'icône de glisser-déposer, si le compositeur les dessine.
fn cursor_elements<R>(
    state: &mut State,
    renderer: &mut R,
    origin: Point<i32, Logical>,
    geo: Rectangle<i32, Logical>,
    scale_int: i32,
    scale: Scale<f64>,
    out: &mut Vec<OutputElement<R>>,
) where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Clone + Send + 'static,
{
    if !state.draw_cursor || !geo.to_f64().contains(state.pointer_location) {
        return;
    }
    let pointer = state.pointer_location - origin.to_f64();

    // Pendant un déplacement ou un redimensionnement, le curseur l'indique.
    let status = match state.cursor_mode {
        CursorMode::Move { .. } => CursorImageStatus::Named(CursorIcon::Grabbing),
        CursorMode::Resize { .. } => CursorImageStatus::Named(CursorIcon::SeResize),
        CursorMode::Normal => state.cursor_status.clone(),
    };
    // Une surface de curseur détruite laisse la place au curseur par défaut.
    let status = match status {
        CursorImageStatus::Surface(surface) if !smithay::utils::IsAlive::alive(&surface) => {
            CursorImageStatus::default_named()
        }
        other => other,
    };

    match status {
        CursorImageStatus::Hidden => {}
        CursorImageStatus::Surface(surface) => {
            let hotspot = with_states(&surface, |states| {
                states
                    .data_map
                    .get::<CursorImageSurfaceData>()
                    .and_then(|data| data.lock().ok().map(|d| d.hotspot))
                    .unwrap_or_default()
            });
            let loc: Point<i32, Physical> = (pointer - hotspot.to_f64()).to_physical(scale).to_i32_round();
            out.extend(render_elements_from_surface_tree::<R, OutputElement<R>>(
                renderer,
                &surface,
                loc,
                scale,
                1.0,
                Kind::Cursor,
            ));
        }
        CursorImageStatus::Named(icon) => {
            let image = state.cursor_theme.image(icon, scale_int);
            let hotspot = Point::<f64, Logical>::from((image.hotspot.0 as f64, image.hotspot.1 as f64));
            let loc: Point<f64, Physical> = (pointer - hotspot).to_physical(scale);
            match MemoryRenderBufferRenderElement::from_buffer(
                renderer,
                loc,
                &image.buffer,
                None,
                None,
                None,
                Kind::Cursor,
            ) {
                Ok(element) => out.push(OutputElement::Memory(element)),
                Err(err) => warn!(?err, "curseur non importé"),
            }
        }
    }

    if let Some(icon) = &state.dnd_icon {
        let loc: Point<i32, Physical> = (pointer + icon.offset.to_f64()).to_physical(scale).to_i32_round();
        out.extend(render_elements_from_surface_tree::<R, OutputElement<R>>(
            renderer,
            &icon.surface,
            loc,
            scale,
            1.0,
            Kind::Unspecified,
        ));
    }
}

/// Envoie les rappels de trame (`wl_surface.frame`) aux surfaces affichées
/// sur une sortie. Les clients masqués n'en reçoivent pas : ils cessent de
/// dessiner.
pub fn send_frames(state: &State, output: &Output) {
    let Some(mon) = state.outputs.iter().find(|e| &e.output == output).map(|e| e.mon) else { return };
    let time = state.start_time.elapsed();
    let throttle = Some(Duration::ZERO);

    let stacking = state.wm.stacking(mon);
    for id in stacking.tiled.iter().chain(&stacking.floating).chain(&stacking.fullscreen) {
        if let Some(managed) = state.windows.get(id) {
            managed.window.send_frame(output, time, throttle, |_, _| Some(output.clone()));
        }
    }
    let map = layer_map_for_output(output);
    for layer in map.layers() {
        layer.send_frame(output, time, throttle, |_, _| Some(output.clone()));
    }
    if let CursorImageStatus::Surface(surface) = &state.cursor_status {
        send_frames_surface_tree(surface, output, time, throttle, |_, _| Some(output.clone()));
    }
    if let Some(icon) = &state.dnd_icon {
        send_frames_surface_tree(&icon.surface, output, time, throttle, |_, _| Some(output.clone()));
    }
}
