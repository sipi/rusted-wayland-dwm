//! `zwlr_layer_shell_v1` : panneaux, lanceurs, fonds d'écran, notifications.

use smithay::delegate_layer_shell;
use smithay::desktop::{layer_map_for_output, LayerSurface};
use smithay::output::Output;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::wayland::shell::wlr_layer::{
    Layer, LayerSurface as WlrLayerSurface, WlrLayerShellHandler, WlrLayerShellState,
};
use smithay::wayland::shell::xdg::PopupSurface;
use tracing::warn;

use crate::state::State;

impl WlrLayerShellHandler for State {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.layer_shell_state
    }

    fn new_layer_surface(
        &mut self,
        surface: WlrLayerSurface,
        wl_output: Option<WlOutput>,
        _layer: Layer,
        namespace: String,
    ) {
        // Sans sortie demandée : celle du moniteur sélectionné.
        let output = wl_output.as_ref().and_then(Output::from_resource).or_else(|| {
            let selected = self.wm.selected_monitor().map(|m| m.id);
            selected
                .and_then(|mon| self.output_for_monitor(mon).cloned())
                .or_else(|| self.outputs.first().map(|e| e.output.clone()))
        });
        let Some(output) = output else {
            surface.send_close();
            return;
        };
        let mut map = layer_map_for_output(&output);
        if let Err(err) = map.map_layer(&LayerSurface::new(surface, namespace)) {
            warn!(?err, "surface layer-shell refusée");
        }
    }

    fn layer_destroyed(&mut self, surface: WlrLayerSurface) {
        let found = self.outputs.iter().find_map(|e| {
            let map = layer_map_for_output(&e.output);
            let layer = map.layers().find(|l| l.layer_surface() == &surface).cloned();
            layer.map(|layer| (e.output.clone(), layer))
        });
        if let Some((output, layer)) = found {
            layer_map_for_output(&output).unmap_layer(&layer);
            // Les zones réservées et le focus clavier ont pu changer.
            self.output_changed(&output);
        }
    }

    fn new_popup(&mut self, _parent: WlrLayerSurface, _popup: PopupSurface) {
        // Le popup est déjà suivi par `XdgShellHandler::new_popup`.
    }
}

delegate_layer_shell!(State);
