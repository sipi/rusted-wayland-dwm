//! `wl_compositor`, `wl_subcompositor`, `wl_shm` : cycle de vie des surfaces.

use smithay::backend::renderer::utils::{on_commit_buffer_handler, with_renderer_surface_state};
use smithay::desktop::{layer_map_for_output, PopupKind, WindowSurfaceType};
use smithay::input::pointer::{CursorImageStatus, CursorImageSurfaceData};
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::Client;
use smithay::wayland::buffer::BufferHandler;
use smithay::wayland::compositor::{
    get_parent, is_sync_subsurface, with_states, CompositorClientState, CompositorHandler, CompositorState,
    SurfaceAttributes,
};
use smithay::wayland::shell::wlr_layer::LayerSurfaceData;
use smithay::wayland::shell::xdg::SurfaceCachedState;
use smithay::wayland::shm::{ShmHandler, ShmState};
use smithay::{delegate_compositor, delegate_shm};

use crate::state::{ClientState, PendingWindow, State};

impl BufferHandler for State {
    fn buffer_destroyed(&mut self, _buffer: &WlBuffer) {}
}

impl CompositorHandler for State {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client.get_data::<ClientState>().expect("tout client porte un ClientState").compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        self.popups.commit(surface);
        self.adjust_cursor_hotspot(surface);

        if is_sync_subsurface(surface) {
            return;
        }
        let mut root = surface.clone();
        while let Some(parent) = get_parent(&root) {
            root = parent;
        }
        let is_root = &root == surface;

        // Commit initial d'un toplevel : il devient un client (DA-11).
        if is_root {
            let pending =
                self.unmapped.iter().position(|p| p.window.toplevel().is_some_and(|t| t.wl_surface() == surface));
            if let Some(pos) = pending {
                let pending = self.unmapped.remove(pos);
                self.manage_window(pending);
                return;
            }
        }

        if let Some(id) = self.client_for_surface(&root) {
            if let Some(managed) = self.windows.get(&id) {
                managed.window.on_commit();
            }
            if is_root {
                self.toplevel_committed(id, surface);
            }
            return;
        }

        if let Some(PopupKind::Xdg(popup)) = self.popups.find_popup(surface) {
            if !popup.is_initial_configure_sent() {
                // La configuration initiale est toujours permise.
                let _ = popup.send_configure();
            }
            return;
        }

        self.layer_committed(surface);
    }
}

impl State {
    /// Commit de la surface racine d'un client géré.
    fn toplevel_committed(&mut self, id: u32, surface: &WlSurface) {
        let has_buffer = with_renderer_surface_state(surface, |s| s.buffer().is_some()).unwrap_or(false);
        let Some(managed) = self.windows.get_mut(&id) else { return };

        if has_buffer {
            let first = !managed.mapped;
            managed.mapped = true;
            let geo = managed.window.geometry();
            let (min, max) = with_states(surface, |states| {
                let mut guard = states.cached_state.get::<SurfaceCachedState>();
                let cached = guard.current();
                (cached.min_size, cached.max_size)
            });
            self.wm.set_size_limits(id, (min.w, min.h), (max.w, max.h));
            // Premier tampon d'un client flottant : sa taille naturelle est
            // connue, il peut être centré.
            let placed = self.wm.set_natural_size(id, geo.size.w, geo.size.h);
            if first || placed {
                self.sync();
            }
        } else if managed.mapped {
            // Tampon nul : le client se démappe. Il redevient un toplevel
            // « neuf », qui devra refaire son commit initial.
            if let Some(managed) = self.unmanage_window(id) {
                if let Some(toplevel) = managed.window.toplevel() {
                    toplevel.reset_initial_configure_sent();
                }
                self.unmapped.push(PendingWindow { window: managed.window, fullscreen: false });
            }
        }
    }

    /// Commit d'une surface *layer-shell* : configuration initiale, puis mise
    /// à jour des zones réservées.
    fn layer_committed(&mut self, surface: &WlSurface) {
        let output = self
            .outputs
            .iter()
            .map(|e| e.output.clone())
            .find(|o| layer_map_for_output(o).layer_for_surface(surface, WindowSurfaceType::TOPLEVEL).is_some());
        let Some(output) = output else { return };

        let initial_configure_sent = with_states(surface, |states| {
            states
                .data_map
                .get::<LayerSurfaceData>()
                .and_then(|data| data.lock().ok().map(|d| d.initial_configure_sent))
                .unwrap_or(true)
        });
        {
            let mut map = layer_map_for_output(&output);
            // Tenir compte de la taille demandée avant de configurer.
            map.arrange();
            if !initial_configure_sent {
                if let Some(layer) = map.layer_for_surface(surface, WindowSurfaceType::TOPLEVEL) {
                    layer.layer_surface().send_configure();
                }
            }
        }
        self.output_changed(&output);
    }

    /// Une surface de curseur peut déplacer son point chaud par `attach(dx, dy)`.
    fn adjust_cursor_hotspot(&mut self, surface: &WlSurface) {
        if !matches!(&self.cursor_status, CursorImageStatus::Surface(s) if s == surface) {
            return;
        }
        with_states(surface, |states| {
            let Some(attrs) = states.data_map.get::<CursorImageSurfaceData>() else { return };
            let delta = states.cached_state.get::<SurfaceAttributes>().current().buffer_delta.take();
            if let (Some(delta), Ok(mut attrs)) = (delta, attrs.lock()) {
                attrs.hotspot -= delta;
            }
        });
    }
}

impl ShmHandler for State {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

delegate_compositor!(State);
delegate_shm!(State);
