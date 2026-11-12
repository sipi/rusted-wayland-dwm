//! `xdg_wm_base` (toplevels, popups) et `zxdg_decoration_manager_v1`.

use smithay::desktop::{
    find_popup_root_surface, get_popup_toplevel_coords, layer_map_for_output, PopupKeyboardGrab, PopupKind,
    PopupPointerGrab, PopupUngrabStrategy, Window, WindowSurfaceType,
};
use smithay::input::pointer::Focus;
use smithay::input::Seat;
use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode;
use smithay::reexports::wayland_server::protocol::{wl_output, wl_seat};
use smithay::utils::{Point, Rectangle, Serial};
use smithay::wayland::compositor::with_states;
use smithay::wayland::shell::xdg::decoration::XdgDecorationHandler;
use smithay::wayland::shell::xdg::{
    PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState, XdgToplevelSurfaceData,
};
use smithay::{delegate_xdg_decoration, delegate_xdg_shell};
use tracing::warn;

use crate::state::{PendingWindow, State};

impl XdgShellHandler for State {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        // Rien n'est configuré ici : le client sera géré à son commit
        // initial, quand son app_id, son titre et son parent seront connus.
        surface.with_pending_state(|state| state.decoration_mode = Some(Mode::ServerSide));
        self.unmapped.push(PendingWindow { window: Window::new_wayland_window(surface), fullscreen: false });
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        self.unmapped.retain(|p| p.window.toplevel() != Some(&surface));
        if let Some(id) = self.client_for_surface(surface.wl_surface()) {
            self.unmanage_window(id);
        }
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        self.unconstrain_popup(&surface);
        if let Err(err) = self.popups.track_popup(PopupKind::from(surface)) {
            warn!(?err, "popup non suivi");
        }
    }

    fn reposition_request(&mut self, surface: PopupSurface, positioner: PositionerState, token: u32) {
        surface.with_pending_state(|state| {
            state.geometry = positioner.get_geometry();
            state.positioner = positioner;
        });
        self.unconstrain_popup(&surface);
        surface.send_repositioned(token);
    }

    fn grab(&mut self, surface: PopupSurface, seat: wl_seat::WlSeat, serial: Serial) {
        let Some(seat) = Seat::<State>::from_resource(&seat) else { return };
        let kind = PopupKind::Xdg(surface);
        let Ok(root) = find_popup_root_surface(&kind) else { return };
        let Ok(mut grab) = self.popups.grab_popup(root, kind, &seat, serial) else { return };

        if let Some(keyboard) = seat.get_keyboard() {
            if keyboard.is_grabbed()
                && !(keyboard.has_grab(serial) || keyboard.has_grab(grab.previous_serial().unwrap_or(serial)))
            {
                grab.ungrab(PopupUngrabStrategy::All);
                return;
            }
            keyboard.set_focus(self, grab.current_grab(), serial);
            keyboard.set_grab(self, PopupKeyboardGrab::new(&grab), serial);
        }
        if let Some(pointer) = seat.get_pointer() {
            if pointer.is_grabbed()
                && !(pointer.has_grab(serial)
                    || pointer.has_grab(grab.previous_serial().unwrap_or_else(|| grab.serial())))
            {
                grab.ungrab(PopupUngrabStrategy::All);
                return;
            }
            pointer.set_grab(self, PopupPointerGrab::new(&grab), serial, Focus::Keep);
        }
    }

    // Déplacement et redimensionnement à l'initiative du client : ignorés,
    // comme dans dwl (pas de décorations côté client). `move_request` et
    // `resize_request` gardent donc leur implémentation par défaut (vide).

    fn fullscreen_request(&mut self, surface: ToplevelSurface, _output: Option<wl_output::WlOutput>) {
        self.set_fullscreen_request(&surface, true);
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        self.set_fullscreen_request(&surface, false);
    }

    fn maximize_request(&mut self, surface: ToplevelSurface) {
        // Non pris en charge, mais le protocole exige une configuration.
        if surface.is_initial_configure_sent() {
            surface.send_configure();
        }
    }

    fn title_changed(&mut self, surface: ToplevelSurface) {
        if let Some(id) = self.client_for_surface(surface.wl_surface()) {
            let title = toplevel_strings(&surface).0;
            self.wm.set_title(id, &title);
        }
    }

    fn app_id_changed(&mut self, surface: ToplevelSurface) {
        if let Some(id) = self.client_for_surface(surface.wl_surface()) {
            let app_id = toplevel_strings(&surface).1;
            self.wm.set_app_id(id, &app_id);
        }
    }
}

/// Titre et app_id courants d'un toplevel.
fn toplevel_strings(surface: &ToplevelSurface) -> (String, String) {
    with_states(surface.wl_surface(), |states| {
        states
            .data_map
            .get::<XdgToplevelSurfaceData>()
            .and_then(|data| data.lock().ok().map(|d| (d.title.clone(), d.app_id.clone())))
            .map(|(title, app_id)| (title.unwrap_or_default(), app_id.unwrap_or_default()))
            .unwrap_or_default()
    })
}

impl State {
    fn set_fullscreen_request(&mut self, surface: &ToplevelSurface, on: bool) {
        if let Some(id) = self.client_for_surface(surface.wl_surface()) {
            self.wm.set_fullscreen(id, on);
            self.sync();
            // Le protocole attend une configuration même sans changement.
            if !surface.has_pending_changes() && surface.is_initial_configure_sent() {
                surface.send_configure();
            }
        } else if let Some(pending) = self.unmapped.iter_mut().find(|p| p.window.toplevel() == Some(surface)) {
            pending.fullscreen = on;
        }
    }

    /// Contraint un popup à rester dans l'écran de son parent.
    fn unconstrain_popup(&self, popup: &PopupSurface) {
        let kind = PopupKind::Xdg(popup.clone());
        let Ok(root) = find_popup_root_surface(&kind) else { return };

        // Zone cible, exprimée par rapport à l'origine de la géométrie du parent.
        let mut target = if let Some(client) = self.client_for_surface(&root).and_then(|id| self.wm.client(id)) {
            let Some(screen) = self.wm.monitor(client.mon).map(|m| m.screen) else { return };
            let inner = client.inner();
            Rectangle::new(Point::from((screen.x - inner.x, screen.y - inner.y)), (screen.w, screen.h).into())
        } else {
            let found = self.outputs.iter().find_map(|e| {
                let map = layer_map_for_output(&e.output);
                let layer = map.layer_for_surface(&root, WindowSurfaceType::TOPLEVEL)?;
                let layer_geo = map.layer_geometry(layer)?;
                let size = self.space.output_geometry(&e.output)?.size;
                Some(Rectangle::new(Point::from((-layer_geo.loc.x, -layer_geo.loc.y)), size))
            });
            let Some(target) = found else { return };
            target
        };
        target.loc -= get_popup_toplevel_coords(&kind);

        popup.with_pending_state(|state| {
            state.geometry = state.positioner.get_unconstrained_geometry(target);
        });
    }
}

impl XdgDecorationHandler for State {
    // Les décorations sont toujours côté serveur : la seule décoration de
    // rwdwm est la bordure, dessinée par le compositeur.

    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        toplevel.with_pending_state(|state| state.decoration_mode = Some(Mode::ServerSide));
    }

    fn request_mode(&mut self, toplevel: ToplevelSurface, _mode: Mode) {
        toplevel.with_pending_state(|state| state.decoration_mode = Some(Mode::ServerSide));
        if toplevel.is_initial_configure_sent() {
            toplevel.send_pending_configure();
        }
    }

    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        toplevel.with_pending_state(|state| state.decoration_mode = Some(Mode::ServerSide));
        if toplevel.is_initial_configure_sent() {
            toplevel.send_pending_configure();
        }
    }
}

delegate_xdg_shell!(State);
delegate_xdg_decoration!(State);
