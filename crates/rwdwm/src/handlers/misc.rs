//! Petits protocoles : activation, dmabuf, viewporter, tampon d'un pixel.

use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::compositor::get_parent;
use smithay::wayland::dmabuf::{DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier};
use smithay::wayland::xdg_activation::{
    XdgActivationHandler, XdgActivationState, XdgActivationToken, XdgActivationTokenData,
};
use smithay::{delegate_dmabuf, delegate_single_pixel_buffer, delegate_viewporter, delegate_xdg_activation};

use crate::backend::BackendData;
use crate::state::State;

impl XdgActivationHandler for State {
    fn activation_state(&mut self) -> &mut XdgActivationState {
        &mut self.xdg_activation_state
    }

    fn request_activation(
        &mut self,
        _token: XdgActivationToken,
        token_data: XdgActivationTokenData,
        surface: WlSurface,
    ) {
        // Un client ne vole jamais le focus : une demande d'activation le
        // marque urgent (la barre et la bordure le signalent), comme dwl.
        if token_data.timestamp.elapsed().as_secs() >= 10 {
            return;
        }
        let mut root = surface;
        while let Some(parent) = get_parent(&root) {
            root = parent;
        }
        if let Some(id) = self.client_for_surface(&root) {
            self.wm.set_urgent(id, true);
        }
    }
}

impl DmabufHandler for State {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.dmabuf_state
    }

    fn dmabuf_imported(&mut self, _global: &DmabufGlobal, dmabuf: Dmabuf, notifier: ImportNotifier) {
        let mut backend = std::mem::replace(&mut self.backend, BackendData::None);
        let imported = backend.import_dmabuf(&dmabuf);
        self.backend = backend;
        if imported {
            let _ = notifier.successful::<State>();
        } else {
            notifier.failed();
        }
    }
}

delegate_xdg_activation!(State);
delegate_dmabuf!(State);
delegate_viewporter!(State);
delegate_single_pixel_buffer!(State);
