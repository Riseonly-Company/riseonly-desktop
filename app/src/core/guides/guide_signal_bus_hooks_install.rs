use rise_energy::guide_signal_bus_hooks::GuideSignalBusHooks;

use crate::core::guides::guide_signal_bus::GuideSignalBus;

pub(crate) fn install_guide_signal_bus_hooks() {
    let _ = GuideSignalBusHooks::set(GuideSignalBusHooks { emit: GuideSignalBus::emit });
}
