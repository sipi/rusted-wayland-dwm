//! Presse-papiers : `wl_data_device`, sélection primaire, `wlr-data-control`.

use std::os::fd::OwnedFd;

use smithay::input::pointer::{CursorImageStatus, CursorImageSurfaceData};
use smithay::input::Seat;
use smithay::reexports::wayland_server::protocol::wl_data_source::WlDataSource;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::Point;
use smithay::wayland::compositor::with_states;
use smithay::wayland::selection::data_device::{
    ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler,
};
use smithay::wayland::selection::primary_selection::{PrimarySelectionHandler, PrimarySelectionState};
use smithay::wayland::selection::wlr_data_control::{DataControlHandler, DataControlState};
use smithay::wayland::selection::SelectionHandler;
use smithay::{delegate_data_control, delegate_data_device, delegate_primary_selection};

use crate::state::{DndIcon, State};

impl SelectionHandler for State {
    type SelectionUserData = ();
}

impl DataDeviceHandler for State {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}

impl ClientDndGrabHandler for State {
    fn started(&mut self, _source: Option<WlDataSource>, icon: Option<WlSurface>, _seat: Seat<Self>) {
        // L'icône est dessinée là où se trouvait l'image du curseur.
        let offset = match &self.cursor_status {
            CursorImageStatus::Surface(surface) => with_states(surface, |states| {
                states
                    .data_map
                    .get::<CursorImageSurfaceData>()
                    .and_then(|data| data.lock().ok().map(|d| Point::from((-d.hotspot.x, -d.hotspot.y))))
                    .unwrap_or_default()
            }),
            _ => Point::default(),
        };
        self.dnd_icon = icon.map(|surface| DndIcon { surface, offset });
    }

    fn dropped(&mut self, _target: Option<WlSurface>, _validated: bool, _seat: Seat<Self>) {
        self.dnd_icon = None;
    }
}

impl ServerDndGrabHandler for State {
    fn send(&mut self, _mime_type: String, _fd: OwnedFd, _seat: Seat<Self>) {
        // Le compositeur n'est jamais la source d'un glisser-déposer.
    }
}

impl PrimarySelectionHandler for State {
    fn primary_selection_state(&self) -> &PrimarySelectionState {
        &self.primary_selection_state
    }
}

impl DataControlHandler for State {
    fn data_control_state(&self) -> &DataControlState {
        &self.data_control_state
    }
}

delegate_data_device!(State);
delegate_primary_selection!(State);
delegate_data_control!(State);
