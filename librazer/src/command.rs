use crate::device::Device;
use crate::error::{RazerError, Result};
use crate::packet::Packet;
use crate::types::{
    BatteryCare, Cluster, CpuBoost, FanMode, FanZone, GpuBoost, LightsAlwaysOn, LogoMode,
    MaxFanSpeedMode, PerfMode, ThermalZone,
};
use log::{debug, trace};

// USB HID command codes - see data/README.md for protocol details
mod cmd {
    // Performance mode commands
    pub const SET_PERF_MODE: u16 = 0x0d02;
    pub const GET_PERF_MODE: u16 = 0x0d82;
    pub const SET_BOOST: u16 = 0x0d07;
    pub const GET_BOOST: u16 = 0x0d87;

    // Fan commands
    pub const SET_FAN_RPM: u16 = 0x0d01;
    pub const GET_FAN_RPM: u16 = 0x0d81;
    pub const GET_ACTUAL_FAN_RPM: u16 = 0x0d88;
    pub const SET_MAX_FAN_SPEED: u16 = 0x070f;
    pub const GET_MAX_FAN_SPEED: u16 = 0x078f;

    // Device information
    pub const GET_FIRMWARE_VERSION: u16 = 0x0081;

    // Logo commands
    pub const SET_LOGO_POWER: u16 = 0x0300;
    pub const GET_LOGO_POWER: u16 = 0x0380;
    pub const SET_LOGO_MODE: u16 = 0x0302;
    pub const GET_LOGO_MODE: u16 = 0x0382;

    // Keyboard commands
    pub const SET_KBD_BRIGHTNESS: u16 = 0x0303;
    pub const GET_KBD_BRIGHTNESS: u16 = 0x0383;

    // Lights always on
    pub const SET_LIGHTS_ALWAYS_ON: u16 = 0x0004;
    pub const GET_LIGHTS_ALWAYS_ON: u16 = 0x0084;

    // Battery care
    pub const SET_BATTERY_CARE: u16 = 0x0712;
    pub const GET_BATTERY_CARE: u16 = 0x0792;
}

fn send_command(device: &Device, command: u16, args: &[u8]) -> Result<Packet> {
    trace!("Sending command 0x{:04X} with args {:02X?}", command, args);
    let response = device.send(Packet::new(command, args))?;
    if !response.get_args().starts_with(args) {
        return Err(RazerError::ResponseMismatch);
    }
    Ok(response)
}

fn set_perf_mode_internal(device: &Device, perf_mode: PerfMode, fan_mode: FanMode) -> Result<()> {
    if (fan_mode == FanMode::Manual) && (perf_mode != PerfMode::Balanced) {
        return Err(RazerError::PreconditionFailed(format!(
            "{:?} allowed only in {:?}",
            fan_mode,
            PerfMode::Balanced
        )));
    }

    if let Some(supported) = device.info.perf_modes {
        if !supported.contains(&perf_mode) {
            return Err(RazerError::PreconditionFailed(format!(
                "{:?} is not supported on this device",
                perf_mode
            )));
        }
    }

    ThermalZone::ALL.into_iter().try_for_each(|zone| {
        send_command(
            device,
            cmd::SET_PERF_MODE,
            &[0x01, zone as u8, perf_mode as u8, fan_mode as u8],
        )
        .map(|_| ())
    })
}

fn set_boost_internal(device: &Device, cluster: Cluster, boost: u8) -> Result<()> {
    let args = &[0, cluster as u8, boost];
    if get_perf_mode(device)? != (PerfMode::Custom, FanMode::Auto) {
        return Err(RazerError::PreconditionFailed(format!(
            "Performance mode must be {:?}",
            PerfMode::Custom
        )));
    }
    let response = device.send(Packet::new(cmd::SET_BOOST, args))?;
    if !response.get_args().starts_with(args) {
        return Err(RazerError::ResponseMismatch);
    }
    Ok(())
}

fn get_boost_internal(device: &Device, cluster: Cluster) -> Result<u8> {
    let response = device.send(Packet::new(cmd::GET_BOOST, &[0, cluster as u8, 0]))?;
    if response.get_args()[1] != cluster as u8 {
        return Err(RazerError::ResponseMismatch);
    }
    Ok(response.get_args()[2])
}

/// Sets the laptop's performance mode (Silent, Balanced, or Custom).
///
/// Fan mode is automatically set to Auto. Use [`set_fan_mode`] to switch to manual fan control.
pub fn set_perf_mode(device: &Device, perf_mode: PerfMode) -> Result<()> {
    debug!("Setting performance mode to {:?}", perf_mode);
    set_perf_mode_internal(device, perf_mode, FanMode::Auto)
}

/// Gets the current performance mode and fan mode.
pub fn get_perf_mode(device: &Device) -> Result<(PerfMode, FanMode)> {
    let response = device.send(Packet::new(
        cmd::GET_PERF_MODE,
        &[0, ThermalZone::Zone1 as u8, 0, 0],
    ))?;
    Ok((
        PerfMode::try_from(response.get_args()[2])?,
        FanMode::try_from(response.get_args()[3])?,
    ))
}

/// Sets the CPU boost level. Requires Custom performance mode.
pub fn set_cpu_boost(device: &Device, boost: CpuBoost) -> Result<()> {
    debug!("Setting CPU boost to {:?}", boost);
    set_boost_internal(device, Cluster::Cpu, boost as u8)
}

/// Sets the GPU boost level. Requires Custom performance mode.
pub fn set_gpu_boost(device: &Device, boost: GpuBoost) -> Result<()> {
    debug!("Setting GPU boost to {:?}", boost);
    set_boost_internal(device, Cluster::Gpu, boost as u8)
}

/// Gets the current CPU boost level.
pub fn get_cpu_boost(device: &Device) -> Result<CpuBoost> {
    CpuBoost::try_from(get_boost_internal(device, Cluster::Cpu)?)
}

/// Gets the current GPU boost level.
pub fn get_gpu_boost(device: &Device) -> Result<GpuBoost> {
    GpuBoost::try_from(get_boost_internal(device, Cluster::Gpu)?)
}

/// Sets the fan speed in RPM. Valid range is 2000-5000.
///
/// Requires Balanced performance mode with Manual fan mode.
pub fn set_fan_rpm(device: &Device, rpm: u16) -> Result<()> {
    if !(2000..=5000).contains(&rpm) {
        return Err(RazerError::PreconditionFailed(format!(
            "RPM must be between 2000 and 5000, got {}",
            rpm
        )));
    }
    if get_perf_mode(device)? != (PerfMode::Balanced, FanMode::Manual) {
        return Err(RazerError::PreconditionFailed(format!(
            "Performance mode must be {:?} and fan mode must be {:?}",
            PerfMode::Balanced,
            FanMode::Manual
        )));
    }
    debug!("Setting fan RPM to {}", rpm);
    [FanZone::Zone1, FanZone::Zone2, FanZone::Zone3, FanZone::Zone4]
        .into_iter()
        .take(device.info.fan_zones as usize)
        .try_for_each(|zone| {
            send_command(
                device,
                cmd::SET_FAN_RPM,
                &[0, zone as u8, (rpm / 100) as u8],
            )
            .map(|_| ())
        })
}

/// Gets the target fan RPM for the specified zone.
pub fn get_fan_rpm(device: &Device, fan_zone: FanZone) -> Result<u16> {
    let response = device.send(Packet::new(cmd::GET_FAN_RPM, &[0, fan_zone as u8, 0]))?;
    if response.get_args()[1] != fan_zone as u8 {
        return Err(RazerError::ResponseMismatch);
    }
    Ok(response.get_args()[2] as u16 * 100)
}

/// Gets the actual (live) fan RPM for the specified zone.
///
/// Unlike [`get_fan_rpm`] which returns the target RPM, this returns the
/// real current speed as reported by the fan controller. Works in any fan mode.
pub fn get_actual_fan_rpm(device: &Device, fan_zone: FanZone) -> Result<u16> {
    let response = device.send(Packet::new(cmd::GET_ACTUAL_FAN_RPM, &[0, fan_zone as u8, 0]))?;
    if response.get_args()[1] != fan_zone as u8 {
        return Err(RazerError::ResponseMismatch);
    }
    Ok(response.get_args()[2] as u16 * 100)
}

/// Gets the device firmware version (e.g., "v01.03").
///
/// The response has data_size=1 but the firmware ASCII string is at args[4..8]
/// in the raw buffer (e.g., `[1, 0, 0, 0, 0x30, 0x31, 0x30, 0x33]` = "0103").
pub fn get_firmware_version(device: &Device) -> Result<String> {
    let response = device.send(Packet::new(cmd::GET_FIRMWARE_VERSION, &[0]))?;
    let args = response.get_raw_args();
    let raw: String = args
        .iter()
        .filter(|b| b.is_ascii_digit())
        .map(|b| *b as char)
        .collect();
    if raw.len() >= 4 {
        Ok(format!("v{}.{}", &raw[..2], &raw[2..4]))
    } else if !raw.is_empty() {
        Ok(raw)
    } else {
        Err(RazerError::ResponseMismatch)
    }
}

/// Enables or disables max fan speed mode. Requires Custom performance mode.
pub fn set_max_fan_speed_mode(device: &Device, mode: MaxFanSpeedMode) -> Result<()> {
    if get_perf_mode(device)?.0 != PerfMode::Custom {
        return Err(RazerError::PreconditionFailed(format!(
            "Performance mode must be {:?}",
            PerfMode::Custom
        )));
    }
    send_command(device, cmd::SET_MAX_FAN_SPEED, &[mode as u8]).map(|_| ())
}

/// Gets the current max fan speed mode setting.
pub fn get_max_fan_speed_mode(device: &Device) -> Result<MaxFanSpeedMode> {
    device
        .send(Packet::new(cmd::GET_MAX_FAN_SPEED, &[0]))?
        .get_args()[0]
        .try_into()
}

/// Sets the fan mode to Auto or Manual. Requires Balanced performance mode.
pub fn set_fan_mode(device: &Device, mode: FanMode) -> Result<()> {
    if get_perf_mode(device)?.0 != PerfMode::Balanced {
        return Err(RazerError::PreconditionFailed(format!(
            "Performance mode must be {:?}",
            PerfMode::Balanced
        )));
    }
    set_perf_mode_internal(device, PerfMode::Balanced, mode)
}

fn set_logo_power(device: &Device, mode: LogoMode) -> Result<Packet> {
    match mode {
        LogoMode::Off => send_command(device, cmd::SET_LOGO_POWER, &[1, 4, 0]),
        LogoMode::Static | LogoMode::Breathing => {
            send_command(device, cmd::SET_LOGO_POWER, &[1, 4, 1])
        }
    }
}

fn set_logo_mode_internal(device: &Device, mode: LogoMode) -> Result<Packet> {
    match mode {
        LogoMode::Static => send_command(device, cmd::SET_LOGO_MODE, &[1, 4, 0]),
        LogoMode::Breathing => send_command(device, cmd::SET_LOGO_MODE, &[1, 4, 2]),
        _ => Err(RazerError::Other("Invalid logo mode".to_string())),
    }
}

fn get_logo_power(device: &Device) -> Result<bool> {
    match device
        .send(Packet::new(cmd::GET_LOGO_POWER, &[1, 4, 0]))?
        .get_args()[2]
    {
        0 => Ok(false),
        1 => Ok(true),
        v => Err(RazerError::InvalidValue {
            value: v,
            type_name: "LogoPower",
        }),
    }
}

fn get_logo_mode_internal(device: &Device) -> Result<LogoMode> {
    match device
        .send(Packet::new(cmd::GET_LOGO_MODE, &[1, 4, 0]))?
        .get_args()[2]
    {
        0 => Ok(LogoMode::Static),
        2 => Ok(LogoMode::Breathing),
        v => Err(RazerError::InvalidValue {
            value: v,
            type_name: "LogoMode",
        }),
    }
}

/// Gets the current lid logo mode (Off, Static, or Breathing).
pub fn get_logo_mode(device: &Device) -> Result<LogoMode> {
    let power = get_logo_power(device)?;
    match power {
        true => get_logo_mode_internal(device),
        false => Ok(LogoMode::Off),
    }
}

/// Sets the lid logo mode (Off, Static, or Breathing).
pub fn set_logo_mode(device: &Device, mode: LogoMode) -> Result<()> {
    debug!("Setting logo mode to {:?}", mode);
    if mode != LogoMode::Off {
        set_logo_mode_internal(device, mode)?;
    }
    set_logo_power(device, mode)?;
    Ok(())
}

/// Gets the current keyboard backlight brightness (0-255).
pub fn get_keyboard_brightness(device: &Device) -> Result<u8> {
    let response = device.send(Packet::new(cmd::GET_KBD_BRIGHTNESS, &[1, 5, 0]))?;
    if response.get_args()[1] != 5 {
        return Err(RazerError::ResponseMismatch);
    }
    Ok(response.get_args()[2])
}

/// Sets the keyboard backlight brightness (0-255).
pub fn set_keyboard_brightness(device: &Device, brightness: u8) -> Result<()> {
    debug!("Setting keyboard brightness to {}", brightness);
    let args = &[1, 5, brightness];
    let response = device.send(Packet::new(cmd::SET_KBD_BRIGHTNESS, args))?;
    if !response.get_args().starts_with(args) {
        return Err(RazerError::ResponseMismatch);
    }
    Ok(())
}

/// Gets whether lights stay on when the laptop is closed/sleeping.
pub fn get_lights_always_on(device: &Device) -> Result<LightsAlwaysOn> {
    device
        .send(Packet::new(cmd::GET_LIGHTS_ALWAYS_ON, &[0, 0]))?
        .get_args()[0]
        .try_into()
}

/// Sets whether lights stay on when the laptop is closed/sleeping.
pub fn set_lights_always_on(device: &Device, lights_always_on: LightsAlwaysOn) -> Result<()> {
    let args = &[lights_always_on as u8, 0];
    let response = device.send(Packet::new(cmd::SET_LIGHTS_ALWAYS_ON, args))?;
    if !response.get_args().starts_with(args) {
        return Err(RazerError::ResponseMismatch);
    }
    Ok(())
}

/// Gets the battery care mode (limits charging to 80% to extend battery life).
pub fn get_battery_care(device: &Device) -> Result<BatteryCare> {
    device
        .send(Packet::new(cmd::GET_BATTERY_CARE, &[0]))?
        .get_args()[0]
        .try_into()
}

/// Sets the battery care mode (limits charging to 80% to extend battery life).
pub fn set_battery_care(device: &Device, mode: BatteryCare) -> Result<()> {
    debug!("Setting battery care to {:?}", mode);
    let args = &[mode as u8];
    let response = device.send(Packet::new(cmd::SET_BATTERY_CARE, args))?;
    if !response.get_args().starts_with(args) {
        return Err(RazerError::ResponseMismatch);
    }
    Ok(())
}
