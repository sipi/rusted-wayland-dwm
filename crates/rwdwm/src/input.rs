//! Entrées : raccourcis clavier, focus qui suit la souris, clics sur la
//! barre, déplacement et redimensionnement à la souris.

use std::time::Duration;

use rwdwm_core::action::Action;
use rwdwm_core::bar::{self, BarClick};
use rwdwm_core::config::{Click, Mods, Trigger};
use rwdwm_core::keysyms::XF86_SWITCH_VT_1;
use rwdwm_core::model::{ClientId, MonitorId};
use smithay::backend::input::{
    AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend, InputEvent, KeyState, KeyboardKeyEvent,
    PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
};
use smithay::desktop::utils::under_from_surface_tree;
use smithay::desktop::{layer_map_for_output, PopupManager, WindowSurfaceType};
use smithay::input::keyboard::{FilterResult, Keycode, ModifiersState};
use smithay::input::pointer::{AxisFrame, ButtonEvent, MotionEvent};
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, SERIAL_COUNTER};
use smithay::wayland::shell::wlr_layer::Layer;

use crate::state::{CursorMode, KeyRepeat, LoopData, State};

/// Surface et position (globale) de son origine.
pub type SurfaceHit = (WlSurface, Point<f64, Logical>);

/// Ce qui se trouve sous un point de l'écran.
#[derive(Debug, Clone, PartialEq)]
pub enum Hit {
    /// Une surface *layer-shell* (ou l'un de ses popups).
    Layer(SurfaceHit),
    /// Un client ; `surface` est absent sur la bordure ou hors de la zone
    /// d'entrée du client.
    Client {
        /// Le client.
        id: ClientId,
        /// La surface précise, s'il y en a une.
        surface: Option<SurfaceHit>,
    },
    /// La barre d'un moniteur.
    Bar {
        /// Le moniteur.
        mon: MonitorId,
        /// Abscisse dans la barre.
        x: i32,
        /// Largeur de la barre.
        width: i32,
    },
    /// Le fond.
    Root,
}

impl Hit {
    /// Le client touché.
    pub fn client(&self) -> Option<ClientId> {
        match self {
            Hit::Client { id, .. } => Some(*id),
            _ => None,
        }
    }

    /// La surface qui doit recevoir les événements du pointeur.
    pub fn surface(self) -> Option<SurfaceHit> {
        match self {
            Hit::Layer(hit) => Some(hit),
            Hit::Client { surface, .. } => surface,
            _ => None,
        }
    }
}

/// Masque de modificateurs « nettoyé » (sans verrouillages), comme
/// `CLEANMASK` de dwm.
fn mods_of(m: &ModifiersState) -> Mods {
    let mut out = Mods::NONE;
    if m.shift {
        out = out | Mods::SHIFT;
    }
    if m.ctrl {
        out = out | Mods::CTRL;
    }
    if m.alt {
        out = out | Mods::ALT;
    }
    if m.logo {
        out = out | Mods::LOGO;
    }
    out
}

/// Actions répétées tant que la touche reste enfoncée.
fn is_repeatable(action: &Action) -> bool {
    matches!(action, Action::FocusStack(_) | Action::SetMFact(_) | Action::IncNMaster(_))
}

impl State {
    // ------------------------------------------------------------------
    // Test de position
    // ------------------------------------------------------------------

    /// Clients flottants susceptibles d'apparaître sur un moniteur, de
    /// l'avant vers l'arrière : ceux du moniteur, puis ceux des autres (une
    /// fenêtre flottante peut déborder sur l'écran voisin).
    pub fn floating_front_to_back(&self, mon: MonitorId) -> Vec<ClientId> {
        let mut out: Vec<ClientId> = self.wm.stacking(mon).floating.into_iter().rev().collect();
        for other in self.wm.monitors().iter().filter(|m| m.id != mon) {
            out.extend(self.wm.stacking(other.id).floating.into_iter().rev());
        }
        out
    }

    /// Origine globale de la surface racine d'un client (la géométrie de la
    /// fenêtre peut être décalée dans la surface).
    fn surface_origin(&self, id: ClientId) -> Option<Point<i32, Logical>> {
        let inner = self.wm.client(id)?.inner();
        let geo = self.windows.get(&id)?.window.geometry();
        Some(Point::from((inner.x, inner.y)) - geo.loc)
    }

    /// Surface d'un client sous le point, limitée à la zone allouée.
    fn client_surface_at(&self, id: ClientId, pos: Point<f64, Logical>) -> Option<SurfaceHit> {
        let inner = self.wm.client(id)?.inner();
        if !inner.contains(pos.x.floor() as i32, pos.y.floor() as i32) {
            return None;
        }
        let managed = self.windows.get(&id).filter(|m| m.mapped)?;
        let surface = managed.window.toplevel()?.wl_surface();
        let origin = self.surface_origin(id)?;
        under_from_surface_tree(surface, pos, origin, WindowSurfaceType::ALL).map(|(s, loc)| (s, loc.to_f64()))
    }

    /// Popup d'un client sous le point (les popups débordent de la fenêtre).
    fn client_popup_at(&self, id: ClientId, pos: Point<f64, Logical>) -> Option<SurfaceHit> {
        let managed = self.windows.get(&id).filter(|m| m.mapped)?;
        let surface = managed.window.toplevel()?.wl_surface();
        let base = self.surface_origin(id)? + managed.window.geometry().loc;
        PopupManager::popups_for_surface(surface).find_map(|(popup, location)| {
            let offset = base + location - popup.geometry().loc;
            under_from_surface_tree(popup.wl_surface(), pos, offset, WindowSurfaceType::ALL)
                .map(|(s, loc)| (s, loc.to_f64()))
        })
    }

    /// Surface *layer-shell* d'une couche sous le point.
    fn layer_surface_at(&self, mon: MonitorId, layer: Layer, pos: Point<f64, Logical>) -> Option<SurfaceHit> {
        let output = self.output_for_monitor(mon)?;
        let output_loc = self.space.output_geometry(output)?.loc;
        let map = layer_map_for_output(output);
        let found = map.layers_on(layer).rev().find_map(|l| {
            let loc = output_loc + map.layer_geometry(l)?.loc;
            l.surface_under(pos - loc.to_f64(), WindowSurfaceType::ALL).map(|(s, p)| (s, (p + loc).to_f64()))
        });
        found
    }

    /// Détermine ce qui se trouve sous un point, dans l'ordre d'empilement :
    /// overlay · plein écran · top · popups · flottants · barre · tuilés ·
    /// bottom · background.
    pub fn hit_test(&self, pos: Point<f64, Logical>) -> Hit {
        let (x, y) = (pos.x.floor() as i32, pos.y.floor() as i32);
        let Some(mon) = self.wm.monitors().iter().find(|m| m.screen.contains(x, y)).map(|m| m.id) else {
            return Hit::Root;
        };
        if let Some(hit) = self.layer_surface_at(mon, Layer::Overlay, pos) {
            return Hit::Layer(hit);
        }

        let stacking = self.wm.stacking(mon);
        if let Some(&id) = stacking.fullscreen.last() {
            // Un client en plein écran masque tout le reste du moniteur.
            let surface = self.client_popup_at(id, pos).or_else(|| self.client_surface_at(id, pos));
            return Hit::Client { id, surface };
        }
        if let Some(hit) = self.layer_surface_at(mon, Layer::Top, pos) {
            return Hit::Layer(hit);
        }

        let floating = self.floating_front_to_back(mon);
        let tiled = stacking.tiled.iter().rev().copied();
        for id in floating.iter().copied().chain(tiled.clone()) {
            if let Some(hit) = self.client_popup_at(id, pos) {
                return Hit::Client { id, surface: Some(hit) };
            }
        }
        let contains = |id: &ClientId| self.wm.client(*id).is_some_and(|c| c.geom.contains(x, y));
        if let Some(&id) = floating.iter().find(|id| contains(id)) {
            return Hit::Client { id, surface: self.client_surface_at(id, pos) };
        }
        if let Some(rect) = self.wm.bar_rect(mon).filter(|r| r.contains(x, y)) {
            return Hit::Bar { mon, x: x - rect.x, width: rect.w };
        }
        if let Some(id) = tiled.clone().find(contains) {
            return Hit::Client { id, surface: self.client_surface_at(id, pos) };
        }
        for layer in [Layer::Bottom, Layer::Background] {
            if let Some(hit) = self.layer_surface_at(mon, layer, pos) {
                return Hit::Layer(hit);
            }
        }
        Hit::Root
    }

    // ------------------------------------------------------------------
    // Événements du backend
    // ------------------------------------------------------------------

    /// Traite un événement d'entrée d'un backend Smithay (libinput, winit).
    pub fn process_input_event<I: InputBackend>(&mut self, event: InputEvent<I>) {
        match event {
            InputEvent::Keyboard { event } => {
                self.on_key(event.key_code(), event.state(), Event::time_msec(&event));
            }
            InputEvent::PointerMotion { event } => {
                let delta = event.delta();
                let pos = self.pointer_location + delta;
                self.on_pointer_motion(pos, Event::time_msec(&event));
            }
            InputEvent::PointerMotionAbsolute { event } => {
                // Position absolue : rapportée à l'ensemble des sorties.
                let Some(bounds) = self.outputs_bounds() else { return };
                let pos = event.position_transformed(bounds.size) + bounds.loc.to_f64();
                self.on_pointer_motion(pos, Event::time_msec(&event));
            }
            InputEvent::PointerButton { event } => {
                self.on_pointer_button(event.button_code(), event.state(), Event::time_msec(&event));
            }
            InputEvent::PointerAxis { event } => self.on_pointer_axis::<I>(event),
            _ => {}
        }
    }

    /// Appui ou relâchement d'une touche.
    pub fn on_key(&mut self, keycode: Keycode, key_state: KeyState, time: u32) {
        let Some(keyboard) = self.seat.get_keyboard() else { return };
        let serial = SERIAL_COUNTER.next_serial();
        let raw = keycode.raw();

        // Toute frappe interrompt la répétition en cours.
        self.stop_key_repeat();

        let action =
            keyboard.input::<Option<Action>, _>(self, keycode, key_state, serial, time, |state, mods, handle| {
                if key_state == KeyState::Released {
                    // Le relâchement d'un raccourci n'est pas transmis non plus.
                    return if state.suppressed_keys.remove(&raw) {
                        FilterResult::Intercept(None)
                    } else {
                        FilterResult::Forward
                    };
                }

                // Ctrl+Alt+Fn : changement de terminal virtuel.
                for sym in handle.modified_syms() {
                    let sym = sym.raw();
                    if (XF86_SWITCH_VT_1..XF86_SWITCH_VT_1 + 12).contains(&sym) {
                        state.suppressed_keys.insert(raw);
                        return FilterResult::Intercept(Some(Action::ChVt((sym - XF86_SWITCH_VT_1 + 1) as i32)));
                    }
                }

                let mods = mods_of(mods);
                let syms: Vec<u32> = handle.raw_syms().iter().map(|s| s.raw()).collect();
                let found = state.wm.config().keys.iter().find(|k| {
                    k.mods == mods
                        && match k.trigger {
                            Trigger::Code(code) => code == raw,
                            Trigger::Sym(sym) => syms.contains(&sym),
                        }
                });
                match found {
                    Some(key) => {
                        let action = key.action.clone();
                        state.suppressed_keys.insert(raw);
                        FilterResult::Intercept(Some(action))
                    }
                    None => FilterResult::Forward,
                }
            });

        if let Some(Some(action)) = action {
            self.apply_action(&action);
            if is_repeatable(&action) {
                self.start_key_repeat(raw, action);
            }
        }
    }

    fn start_key_repeat(&mut self, keycode: u32, action: Action) {
        let kb = &self.wm.config().keyboard;
        if kb.repeat_rate <= 0 {
            return;
        }
        let delay = Duration::from_millis(kb.repeat_delay.max(1) as u64);
        let interval = Duration::from_millis((1000 / kb.repeat_rate).max(1) as u64);
        let timer = Timer::from_duration(delay);
        let token = self.loop_handle.insert_source(timer, move |_, _, data: &mut LoopData| {
            data.state.apply_action(&action);
            TimeoutAction::ToDuration(interval)
        });
        if let Ok(token) = token {
            self.key_repeat = Some(KeyRepeat { keycode, token });
        }
    }

    fn stop_key_repeat(&mut self) {
        if let Some(repeat) = self.key_repeat.take() {
            self.loop_handle.remove(repeat.token);
        }
    }

    /// Le pointeur est désormais en `pos` (coordonnées globales).
    pub fn on_pointer_motion(&mut self, pos: Point<f64, Logical>, time: u32) {
        let Some(pointer) = self.seat.get_pointer() else { return };
        let pos = self.clamp_to_outputs(pos);
        self.pointer_location = pos;
        let (x, y) = (pos.x.floor() as i32, pos.y.floor() as i32);

        match self.cursor_mode {
            CursorMode::Move { client, offset } => {
                self.wm.mouse_move(client, x - offset.0, y - offset.1);
                self.sync();
            }
            CursorMode::Resize { client, start, size } => {
                self.wm.mouse_resize(client, size.0 + x - start.0, size.1 + y - start.1);
                self.sync();
            }
            CursorMode::Normal => {
                let hit = self.hit_test(pos);
                let hovered = hit.client();
                let mut changed = false;
                // Le focus suit la souris : seulement quand elle *entre* dans
                // un autre client (EnterNotify de dwm).
                if hovered != self.hovered && !pointer.is_grabbed() {
                    self.hovered = hovered;
                    if let Some(id) = hovered.filter(|_| self.wm.config().sloppy_focus) {
                        if self.wm.selected() != Some(id) {
                            self.wm.focus_client(id);
                            changed = true;
                        }
                    }
                }
                // Changer d'écran sélectionne le moniteur survolé.
                let mon = self.wm.monitors().iter().find(|m| m.screen.contains(x, y)).map(|m| m.id);
                if let Some(mon) = mon.filter(|_| hovered.is_none()) {
                    if self.wm.selected_monitor().map(|m| m.id) != Some(mon) {
                        self.wm.select_monitor(mon);
                        changed = true;
                    }
                }
                if changed {
                    self.sync();
                }
            }
        }

        let under = if self.cursor_mode == CursorMode::Normal { self.hit_test(pos).surface() } else { None };
        let event = MotionEvent { location: pos, serial: SERIAL_COUNTER.next_serial(), time };
        pointer.motion(self, under, &event);
        pointer.frame(self);
    }

    /// Ramène un point dans l'union des sorties.
    fn clamp_to_outputs(&self, pos: Point<f64, Logical>) -> Point<f64, Logical> {
        let (x, y) = (pos.x.floor() as i32, pos.y.floor() as i32);
        let rects: Vec<_> = self.wm.monitors().iter().map(|m| m.screen).collect();
        if rects.is_empty() || rects.iter().any(|r| r.contains(x, y)) {
            return pos;
        }
        // Hors de toute sortie : point le plus proche dans la sortie la plus proche.
        let clamp = |r: &rwdwm_core::Rect| {
            let cx = pos.x.clamp(r.x as f64, (r.right() - 1) as f64);
            let cy = pos.y.clamp(r.y as f64, (r.bottom() - 1) as f64);
            let d = (cx - pos.x).powi(2) + (cy - pos.y).powi(2);
            (d, Point::from((cx, cy)))
        };
        rects.iter().map(clamp).min_by(|a, b| a.0.total_cmp(&b.0)).map_or(pos, |(_, p)| p)
    }

    /// Appui ou relâchement d'un bouton de souris.
    pub fn on_pointer_button(&mut self, button: u32, state: ButtonState, time: u32) {
        let Some(pointer) = self.seat.get_pointer() else { return };
        let serial = SERIAL_COUNTER.next_serial();
        let forward = |this: &mut State| {
            pointer.button(this, &ButtonEvent { button, state, serial, time });
            pointer.frame(this);
        };

        if state == ButtonState::Released {
            if let CursorMode::Move { client, .. } | CursorMode::Resize { client, .. } = self.cursor_mode {
                self.cursor_mode = CursorMode::Normal;
                self.wm.mouse_release(client);
                self.sync();
            }
            if !self.suppressed_buttons.remove(&button) {
                forward(self);
            }
            return;
        }

        if self.cursor_mode != CursorMode::Normal {
            self.suppressed_buttons.insert(button);
            return;
        }
        // Pendant une saisie de popup, les clics vont à la saisie (qui ferme
        // le menu si l'on clique ailleurs).
        if pointer.is_grabbed() {
            forward(self);
            return;
        }

        let (x, y) = (self.pointer_location.x.floor() as i32, self.pointer_location.y.floor() as i32);
        let (click, tag) = match self.hit_test(self.pointer_location) {
            Hit::Layer(_) => {
                forward(self);
                return;
            }
            Hit::Bar { mon, x, width } => {
                self.wm.select_monitor(mon);
                match self.wm.bar_view(mon).map(|view| bar::click_at(&view, width, x)) {
                    Some(BarClick::Tag(i)) => (Click::TagBar, 1 << i),
                    Some(BarClick::LayoutSymbol) => (Click::LtSymbol, 0),
                    Some(BarClick::Status) => (Click::StatusText, 0),
                    Some(BarClick::Title) | None => (Click::WinTitle, 0),
                }
            }
            Hit::Client { id, .. } => {
                self.wm.focus_client(id);
                (Click::ClientWin, 0)
            }
            Hit::Root => {
                if let Some(mon) = self.wm.monitors().iter().find(|m| m.screen.contains(x, y)).map(|m| m.id) {
                    self.wm.select_monitor(mon);
                }
                (Click::RootWin, 0)
            }
        };
        self.sync();

        let mods = self.seat.get_keyboard().map(|k| mods_of(&k.modifier_state())).unwrap_or_default();
        let binding =
            self.wm.config().buttons.iter().find(|b| b.click == click && b.button == button && b.mods == mods);
        match binding.map(|b| b.action.with_tag_if_unset(tag)) {
            Some(action) => {
                self.suppressed_buttons.insert(button);
                self.apply_action(&action);
            }
            None => forward(self),
        }
    }

    fn on_pointer_axis<I: InputBackend>(&mut self, event: I::PointerAxisEvent) {
        let Some(pointer) = self.seat.get_pointer() else { return };
        let source = event.source();
        let mut frame = AxisFrame::new(Event::time_msec(&event)).source(source);
        for axis in [Axis::Horizontal, Axis::Vertical] {
            let v120 = event.amount_v120(axis);
            let amount = event.amount(axis).unwrap_or_else(|| v120.unwrap_or(0.0) * 15.0 / 120.0);
            if amount != 0.0 {
                frame = frame.value(axis, amount);
                if let Some(v120) = v120 {
                    frame = frame.v120(axis, v120 as i32);
                }
            } else if source == AxisSource::Finger && event.amount(axis) == Some(0.0) {
                frame = frame.stop(axis);
            }
        }
        pointer.axis(self, frame);
        pointer.frame(self);
    }
}
