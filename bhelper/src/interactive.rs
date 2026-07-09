use crate::device::BladeDevice;
use crate::display;
use crate::error::{Error, Result};
use crate::settings::SettingValue;
use colored::*;
use dialoguer::{Confirm, Input, Select};
use librazer::command;
use librazer::types::*;

#[derive(Debug, Clone, Copy)]
enum Action {
    PerfMode,
    CpuBoost,
    GpuBoost,
    FanControl,
    MaxFanSpeed,
    KeyboardBrightness,
    LogoMode,
    BatteryCare,
    LightsAlwaysOn,
    Exit,
}

struct MenuItem {
    label: &'static str,
    action: Action,
}

pub fn run() -> Result<()> {
    let device = BladeDevice::detect_with_cache()?;

    loop {
        // Clear screen and print status
        print!("\x1B[2J\x1B[H");
        let state = device.read_state()?;
        println!();
        display::print_status(&device, &state);
        println!();

        // Build menu
        let items = build_menu(&device);
        let labels: Vec<&str> = items.iter().map(|i| i.label).collect();

        let selection = Select::new()
            .with_prompt("What would you like to do?")
            .items(&labels)
            .default(0)
            .interact_opt()
            .map_err(|e| Error::Interactive(e.to_string()))?;

        let Some(idx) = selection else {
            // Escape pressed
            break;
        };

        let action = items[idx].action;
        if matches!(action, Action::Exit) {
            break;
        }

        if let Err(e) = handle_action(&device, action) {
            eprintln!("{} {}", "Error:".red().bold(), e);
        }
    }

    Ok(())
}

fn build_menu(device: &BladeDevice) -> Vec<MenuItem> {
    let mut items = vec![
        MenuItem {
            label: "Performance Mode",
            action: Action::PerfMode,
        },
        MenuItem {
            label: "CPU Boost",
            action: Action::CpuBoost,
        },
        MenuItem {
            label: "GPU Boost",
            action: Action::GpuBoost,
        },
        MenuItem {
            label: "Fan Control",
            action: Action::FanControl,
        },
        MenuItem {
            label: "Max Fan Speed",
            action: Action::MaxFanSpeed,
        },
    ];

    if device.supports("kbd-backlight") {
        items.push(MenuItem {
            label: "Keyboard Brightness",
            action: Action::KeyboardBrightness,
        });
    }

    if device.supports("lid-logo") {
        items.push(MenuItem {
            label: "Logo Mode",
            action: Action::LogoMode,
        });
    }

    if device.supports("battery-care") {
        items.push(MenuItem {
            label: "Battery Care",
            action: Action::BatteryCare,
        });
    }

    if device.supports("lights-always-on") {
        items.push(MenuItem {
            label: "Lights Always On",
            action: Action::LightsAlwaysOn,
        });
    }

    items.push(MenuItem {
        label: "Exit",
        action: Action::Exit,
    });

    items
}

fn handle_action(device: &BladeDevice, action: Action) -> Result<()> {
    match action {
        Action::PerfMode => set_perf_mode(device),
        Action::CpuBoost => set_cpu_boost(device),
        Action::GpuBoost => set_gpu_boost(device),
        Action::FanControl => fan_control_menu(device),
        Action::MaxFanSpeed => set_max_fan_speed(device),
        Action::KeyboardBrightness => set_keyboard_brightness(device),
        Action::LogoMode => set_logo_mode(device),
        Action::BatteryCare => set_battery_care(device),
        Action::LightsAlwaysOn => set_lights_always_on(device),
        Action::Exit => Ok(()),
    }
}

/// Check current perf mode and prompt to switch if needed. Returns false if user declined.
fn ensure_perf_mode(device: &BladeDevice, required: PerfMode) -> Result<bool> {
    let (current, _) = command::get_perf_mode(device.inner())?;
    if current == required {
        return Ok(true);
    }

    let prompt = format!(
        "This requires {:?} mode (currently {:?}). Switch?",
        required, current
    );
    let confirmed = Confirm::new()
        .with_prompt(prompt)
        .default(true)
        .interact()
        .map_err(|e| Error::Interactive(e.to_string()))?;

    if !confirmed {
        return Ok(false);
    }

    command::set_perf_mode(device.inner(), required)?;
    println!("{} Switched to {:?} mode", "✓".green(), required);
    Ok(true)
}

/// Ensure Balanced mode with Manual fan for RPM setting.
fn ensure_manual_fan(device: &BladeDevice) -> Result<bool> {
    let (current_perf, current_fan) = command::get_perf_mode(device.inner())?;

    // First ensure Balanced mode
    if current_perf != PerfMode::Balanced {
        let prompt = format!(
            "Manual fan requires Balanced mode (currently {:?}). Switch?",
            current_perf
        );
        let confirmed = Confirm::new()
            .with_prompt(prompt)
            .default(true)
            .interact()
            .map_err(|e| Error::Interactive(e.to_string()))?;

        if !confirmed {
            return Ok(false);
        }

        command::set_perf_mode(device.inner(), PerfMode::Balanced)?;
        println!("{} Switched to Balanced mode", "✓".green());
    }

    // Then ensure Manual fan mode
    if current_fan != FanMode::Manual || current_perf != PerfMode::Balanced {
        command::set_fan_mode(device.inner(), FanMode::Manual)?;
        println!("{} Fan set to Manual mode", "✓".green());
    }

    Ok(true)
}

fn set_perf_mode(device: &BladeDevice) -> Result<()> {
    let modes: Vec<PerfMode> = if let Some(supported) = device.perf_modes() {
        supported.to_vec()
    } else {
        vec![PerfMode::Silent, PerfMode::Balanced, PerfMode::Custom]
    };

    let current = command::get_perf_mode(device.inner())?.0;
    let labels: Vec<String> = modes
        .iter()
        .map(|m| {
            if *m == current {
                format!("{:?} (current)", m)
            } else {
                format!("{:?}", m)
            }
        })
        .collect();

    let default = modes.iter().position(|m| *m == current).unwrap_or(0);
    let selection = Select::new()
        .with_prompt("Select performance mode")
        .items(&labels)
        .default(default)
        .interact_opt()
        .map_err(|e| Error::Interactive(e.to_string()))?;

    let Some(idx) = selection else { return Ok(()) };
    let chosen = modes[idx];

    device.apply_setting(SettingValue::PerfMode {
        mode: chosen,
        fan_mode: FanMode::Auto,
    })?;
    display::print_setting_changed("Performance Mode", &SettingValue::PerfMode {
        mode: chosen,
        fan_mode: FanMode::Auto,
    });
    Ok(())
}

fn set_cpu_boost(device: &BladeDevice) -> Result<()> {
    if !ensure_perf_mode(device, PerfMode::Custom)? {
        return Ok(());
    }

    let options = [
        CpuBoost::Low,
        CpuBoost::Medium,
        CpuBoost::High,
        CpuBoost::Boost,
        CpuBoost::Overclock,
    ];
    let current = command::get_cpu_boost(device.inner()).ok();
    let labels: Vec<String> = options
        .iter()
        .map(|b| {
            if Some(*b) == current {
                format!("{:?} (current)", b)
            } else {
                format!("{:?}", b)
            }
        })
        .collect();

    let default = current
        .and_then(|c| options.iter().position(|b| *b == c))
        .unwrap_or(0);
    let selection = Select::new()
        .with_prompt("Select CPU boost level")
        .items(&labels)
        .default(default)
        .interact_opt()
        .map_err(|e| Error::Interactive(e.to_string()))?;

    let Some(idx) = selection else { return Ok(()) };
    let value = SettingValue::CpuBoost(options[idx]);
    device.apply_setting(value.clone())?;
    display::print_setting_changed("CPU Boost", &value);
    Ok(())
}

fn set_gpu_boost(device: &BladeDevice) -> Result<()> {
    if !ensure_perf_mode(device, PerfMode::Custom)? {
        return Ok(());
    }

    let options = [GpuBoost::Low, GpuBoost::Medium, GpuBoost::High];
    let current = command::get_gpu_boost(device.inner()).ok();
    let labels: Vec<String> = options
        .iter()
        .map(|b| {
            if Some(*b) == current {
                format!("{:?} (current)", b)
            } else {
                format!("{:?}", b)
            }
        })
        .collect();

    let default = current
        .and_then(|c| options.iter().position(|b| *b == c))
        .unwrap_or(0);
    let selection = Select::new()
        .with_prompt("Select GPU boost level")
        .items(&labels)
        .default(default)
        .interact_opt()
        .map_err(|e| Error::Interactive(e.to_string()))?;

    let Some(idx) = selection else { return Ok(()) };
    let value = SettingValue::GpuBoost(options[idx]);
    device.apply_setting(value.clone())?;
    display::print_setting_changed("GPU Boost", &value);
    Ok(())
}

fn fan_control_menu(device: &BladeDevice) -> Result<()> {
    let items = ["Auto", "Manual (set RPM)", "Back"];

    let selection = Select::new()
        .with_prompt("Fan control")
        .items(&items)
        .default(0)
        .interact_opt()
        .map_err(|e| Error::Interactive(e.to_string()))?;

    match selection {
        Some(0) => {
            // Auto: requires Balanced
            if !ensure_perf_mode(device, PerfMode::Balanced)? {
                return Ok(());
            }
            let value = SettingValue::Fan {
                mode: FanMode::Auto,
                rpm: None,
            };
            device.apply_setting(value.clone())?;
            display::print_setting_changed("Fan", &value);
        }
        Some(1) => {
            // Manual RPM: requires Balanced + Manual
            if !ensure_manual_fan(device)? {
                return Ok(());
            }
            let current_rpm = command::get_fan_rpm(device.inner(), FanZone::Zone1).ok();
            let prompt = if let Some(rpm) = current_rpm {
                format!("Enter fan RPM (2000-5000, current: {})", rpm)
            } else {
                "Enter fan RPM (2000-5000)".to_string()
            };

            let rpm: u16 = Input::new()
                .with_prompt(prompt)
                .validate_with(|input: &u16| {
                    if (2000..=5000).contains(input) {
                        Ok(())
                    } else {
                        Err("RPM must be between 2000 and 5000")
                    }
                })
                .interact_text()
                .map_err(|e| Error::Interactive(e.to_string()))?;

            command::set_fan_rpm(device.inner(), rpm)?;
            let value = SettingValue::Fan {
                mode: FanMode::Manual,
                rpm: Some(rpm),
            };
            display::print_setting_changed("Fan", &value);
        }
        _ => {} // Back or Escape
    }

    Ok(())
}

fn set_max_fan_speed(device: &BladeDevice) -> Result<()> {
    if !ensure_perf_mode(device, PerfMode::Custom)? {
        return Ok(());
    }

    let options = [MaxFanSpeedMode::Enable, MaxFanSpeedMode::Disable];
    let current = command::get_max_fan_speed_mode(device.inner()).ok();
    let labels: Vec<String> = options
        .iter()
        .map(|m| {
            if Some(*m) == current {
                format!("{:?} (current)", m)
            } else {
                format!("{:?}", m)
            }
        })
        .collect();

    let default = current
        .and_then(|c| options.iter().position(|m| *m == c))
        .unwrap_or(0);
    let selection = Select::new()
        .with_prompt("Max fan speed")
        .items(&labels)
        .default(default)
        .interact_opt()
        .map_err(|e| Error::Interactive(e.to_string()))?;

    let Some(idx) = selection else { return Ok(()) };
    let value = SettingValue::MaxFanSpeed(options[idx]);
    device.apply_setting(value.clone())?;
    display::print_setting_changed("Max Fan Speed", &value);
    Ok(())
}

fn set_keyboard_brightness(device: &BladeDevice) -> Result<()> {
    let current = command::get_keyboard_brightness(device.inner()).ok();
    let prompt = if let Some(b) = current {
        format!("Keyboard brightness (0-255, current: {})", b)
    } else {
        "Keyboard brightness (0-255)".to_string()
    };

    let brightness: u8 = Input::new()
        .with_prompt(prompt)
        .interact_text()
        .map_err(|e| Error::Interactive(e.to_string()))?;

    let value = SettingValue::KeyboardBrightness(brightness);
    device.apply_setting(value.clone())?;
    display::print_setting_changed("Keyboard Brightness", &value);
    Ok(())
}

fn set_logo_mode(device: &BladeDevice) -> Result<()> {
    let options = [LogoMode::Off, LogoMode::Static, LogoMode::Breathing];
    let current = command::get_logo_mode(device.inner()).ok();
    let labels: Vec<String> = options
        .iter()
        .map(|m| {
            if Some(*m) == current {
                format!("{:?} (current)", m)
            } else {
                format!("{:?}", m)
            }
        })
        .collect();

    let default = current
        .and_then(|c| options.iter().position(|m| *m == c))
        .unwrap_or(0);
    let selection = Select::new()
        .with_prompt("Select logo mode")
        .items(&labels)
        .default(default)
        .interact_opt()
        .map_err(|e| Error::Interactive(e.to_string()))?;

    let Some(idx) = selection else { return Ok(()) };
    let value = SettingValue::LogoMode(options[idx]);
    device.apply_setting(value.clone())?;
    display::print_setting_changed("Logo Mode", &value);
    Ok(())
}

fn set_battery_care(device: &BladeDevice) -> Result<()> {
    let options = [BatteryCare::Enable, BatteryCare::Disable];
    let current = command::get_battery_care(device.inner()).ok();
    let labels: Vec<String> = options
        .iter()
        .map(|m| {
            if Some(*m) == current {
                format!("{:?} (current)", m)
            } else {
                format!("{:?}", m)
            }
        })
        .collect();

    let default = current
        .and_then(|c| options.iter().position(|m| *m == c))
        .unwrap_or(0);
    let selection = Select::new()
        .with_prompt("Battery care")
        .items(&labels)
        .default(default)
        .interact_opt()
        .map_err(|e| Error::Interactive(e.to_string()))?;

    let Some(idx) = selection else { return Ok(()) };
    let value = SettingValue::BatteryCare(options[idx]);
    device.apply_setting(value.clone())?;
    display::print_setting_changed("Battery Care", &value);
    Ok(())
}

fn set_lights_always_on(device: &BladeDevice) -> Result<()> {
    let options = [LightsAlwaysOn::Enable, LightsAlwaysOn::Disable];
    let current = command::get_lights_always_on(device.inner()).ok();
    let labels: Vec<String> = options
        .iter()
        .map(|m| {
            if Some(*m) == current {
                format!("{:?} (current)", m)
            } else {
                format!("{:?}", m)
            }
        })
        .collect();

    let default = current
        .and_then(|c| options.iter().position(|m| *m == c))
        .unwrap_or(0);
    let selection = Select::new()
        .with_prompt("Lights always on")
        .items(&labels)
        .default(default)
        .interact_opt()
        .map_err(|e| Error::Interactive(e.to_string()))?;

    let Some(idx) = selection else { return Ok(()) };
    let value = SettingValue::LightsAlwaysOn(options[idx]);
    device.apply_setting(value.clone())?;
    display::print_setting_changed("Lights Always On", &value);
    Ok(())
}
