//! `wl_output` et `zxdg_output_manager_v1`.

use smithay::delegate_output;
use smithay::wayland::output::OutputHandler;

use crate::state::State;

impl OutputHandler for State {}

delegate_output!(State);
