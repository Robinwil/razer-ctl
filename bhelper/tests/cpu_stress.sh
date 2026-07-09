#!/usr/bin/env bash
# CPU Stress Test — measures temps, frequency, and power across CPU boost modes
# Requires: stress-ng, sensors (lm_sensors), bhelper
# Optional: turbostat (for deeper MSR-level data)

set -uo pipefail

# ─── Defaults ────────────────────────────────────────────────────────────────

DURATION=120          # seconds per mode
SAMPLE_INTERVAL=2     # seconds between samples
COOLDOWN_TARGET=45    # °C — cool to this before each mode
COOLDOWN_MAX_WAIT=45  # seconds max to wait for cooldown
IDLE_SAMPLES=3        # seconds of idle baseline
MODES=(low medium high boost overclock)
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
RESULTS_DIR="${SCRIPT_DIR}/results"

# Find bhelper binary (udev rule grants USB access, no sudo needed)
BHELPER=""
for candidate in \
    "${SCRIPT_DIR}/../target/release/bhelper" \
    "${SCRIPT_DIR}/../../target/release/bhelper" \
    "$(command -v bhelper 2>/dev/null)"; do
    if [[ -n "$candidate" && -x "$candidate" ]]; then
        BHELPER="$candidate"
        break
    fi
done
[[ -n "$BHELPER" ]] || { echo "ERROR: bhelper binary not found" >&2; exit 1; }
CSV_FILE=""

# ─── Colors ──────────────────────────────────────────────────────────────────

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
DIM='\033[2m'
RESET='\033[0m'

# ─── Parse args ──────────────────────────────────────────────────────────────

usage() {
    echo "Usage: $0 [-d DURATION] [-m MODES] [-c COOLDOWN_TEMP] [-h]"
    echo "  -d  Duration per mode in seconds (default: $DURATION)"
    echo "  -m  Comma-separated modes (default: ${MODES[*]})"
    echo "  -c  Cooldown target temp in °C (default: $COOLDOWN_TARGET)"
    echo "  -h  Show this help"
    exit 0
}

while getopts "d:m:c:h" opt; do
    case $opt in
        d) DURATION=$OPTARG ;;
        m) IFS=',' read -ra MODES <<< "$OPTARG" ;;
        c) COOLDOWN_TARGET=$OPTARG ;;
        h) usage ;;
        *) usage ;;
    esac
done

# ─── Preflight checks ───────────────────────────────────────────────────────

fail() { echo -e "${RED}ERROR:${RESET} $1" >&2; exit 1; }
warn() { echo -e "${YELLOW}WARN:${RESET} $1" >&2; }

# Verify bhelper works
$BHELPER get perf &>/dev/null || fail "bhelper cannot communicate with device. Check permissions / sudo."

command -v stress-ng &>/dev/null || fail "stress-ng not found. Install: sudo pacman -S stress-ng"
command -v sensors &>/dev/null || fail "sensors not found. Install: sudo pacman -S lm_sensors"

# Detect CPU core count
NCPU=$(nproc)

# Detect RAPL energy counter (root-only, optional)
RAPL_PATH=""
for p in /sys/class/powercap/intel-rapl:0/energy_uj /sys/class/powercap/intel-rapl/intel-rapl:0/energy_uj; do
    if [[ -r "$p" ]]; then
        RAPL_PATH="$p"
        break
    fi
done
[[ -n "$RAPL_PATH" ]] || warn "RAPL energy counter not readable (root-only) — power measurement disabled. Run as root to enable."

# Detect hwmon for coretemp
HWMON_PATH=""
for hwmon in /sys/class/hwmon/hwmon*/; do
    if [[ -r "${hwmon}name" ]] && [[ "$(cat "${hwmon}name")" == "coretemp" ]]; then
        HWMON_PATH="$hwmon"
        break
    fi
done
[[ -n "$HWMON_PATH" ]] || fail "coretemp hwmon not found — cannot read CPU temps"

# Detect cpufreq
CPUFREQ_AVAIL=false
[[ -r /sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq ]] && CPUFREQ_AVAIL=true
$CPUFREQ_AVAIL || warn "cpufreq not available — frequency tracking disabled"

# ─── State tracking ─────────────────────────────────────────────────────────

STRESS_PID=""
ORIGINAL_MODE=""

# ─── Cleanup ─────────────────────────────────────────────────────────────────

cleanup() {
    local exit_code=$?
    echo ""
    echo -e "${DIM}Cleaning up...${RESET}"

    # Kill stress-ng if running
    if [[ -n "$STRESS_PID" ]] && kill -0 "$STRESS_PID" 2>/dev/null; then
        kill "$STRESS_PID" 2>/dev/null
        wait "$STRESS_PID" 2>/dev/null || true
    fi

    # Reset to balanced mode with auto fans
    $BHELPER set perf balanced &>/dev/null || true
    $BHELPER set fan auto &>/dev/null || true

    echo -e "${GREEN}Reset to balanced mode with auto fans.${RESET}"
    exit "$exit_code"
}
trap cleanup EXIT INT TERM

# ─── Sensor helpers ──────────────────────────────────────────────────────────

get_pkg_temp() {
    # Package temp is typically temp1_input in coretemp
    local temp
    temp=$(cat "${HWMON_PATH}temp1_input" 2>/dev/null) || return 1
    echo $(( temp / 1000 ))
}

get_core_temps() {
    # Returns space-separated core temps in °C
    # Core temps are temp2_input, temp3_input, ... in coretemp
    local temps=()
    for f in "${HWMON_PATH}"temp*_input; do
        [[ "$f" == *"temp1_input" ]] && continue  # skip package temp
        local t
        t=$(cat "$f" 2>/dev/null) || continue
        temps+=( $(( t / 1000 )) )
    done
    echo "${temps[*]}"
}

get_max_core_temp() {
    local temps max=0
    temps=$(get_core_temps)
    for t in $temps; do
        (( t > max )) && max=$t
    done
    echo "$max"
}

get_avg_core_temp() {
    local temps sum=0 count=0
    temps=$(get_core_temps)
    for t in $temps; do
        (( sum += t ))
        (( count++ ))
    done
    if (( count > 0 )); then
        echo $(( sum / count ))
    else
        echo 0
    fi
}

get_avg_freq_mhz() {
    $CPUFREQ_AVAIL || { echo 0; return; }
    local sum=0 count=0
    for f in /sys/devices/system/cpu/cpu*/cpufreq/scaling_cur_freq; do
        local khz
        khz=$(cat "$f" 2>/dev/null) || continue
        (( sum += khz ))
        (( count++ ))
    done
    if (( count > 0 )); then
        echo $(( sum / count / 1000 ))
    else
        echo 0
    fi
}

get_rapl_energy_uj() {
    [[ -n "$RAPL_PATH" ]] || { echo 0; return; }
    cat "$RAPL_PATH" 2>/dev/null || echo 0
}

get_fan_rpms() {
    # Get fan RPMs from bhelper JSON output
    local json rpms
    json=$($BHELPER --json get fan 2>/dev/null) || { echo "0,0,0,0"; return; }
    # Parse "Manual @ NNNN RPM" or just get the rpm value
    # Use bhelper status --json for fan_rpms array
    json=$($BHELPER --json status 2>/dev/null) || { echo "0,0,0,0"; return; }
    rpms=$(echo "$json" | python3 -c "
import sys, json
data = json.load(sys.stdin)
rpms = data.get('state', {}).get('fan_rpms', [0,0,0,0])
print(','.join(str(r) for r in rpms))
" 2>/dev/null) || rpms="0,0,0,0"
    echo "$rpms"
}

# ─── Power calculation ──────────────────────────────────────────────────────

calc_power_watts() {
    local energy_before=$1 energy_after=$2 elapsed_us=$3
    if (( elapsed_us == 0 )) || [[ -z "$RAPL_PATH" ]]; then
        echo "0.0"
        return
    fi
    # energy is in microjoules, elapsed in microseconds → watts = ΔE(uJ) / Δt(us)
    # Handle counter wraparound (32-bit or 64-bit)
    local delta=$(( energy_after - energy_before ))
    if (( delta < 0 )); then
        # Assume 32-bit wraparound
        delta=$(( delta + 4294967296 ))
    fi
    # watts = uJ / (us) = uJ / (s * 1000000) → watts = delta / elapsed_us
    python3 -c "print(f'{$delta / $elapsed_us:.1f}')" 2>/dev/null || echo "0.0"
}

# ─── Cooldown ────────────────────────────────────────────────────────────────

cooldown() {
    local target=$1 max_wait=$2 mode_name=$3
    local start_temp elapsed=0

    start_temp=$(get_pkg_temp)
    if (( start_temp <= target )); then
        echo -e "${DIM}[${mode_name}] already at ${start_temp}°C ✓${RESET}"
        return 0
    fi

    # Set fans to max for cooling
    $BHELPER set fan manual 5000 &>/dev/null || true

    echo -ne "${DIM}[${mode_name}] cooling to ${target}°C... ${start_temp}°C${RESET}"

    while (( elapsed < max_wait )); do
        sleep 1
        (( elapsed++ ))
        local current
        current=$(get_pkg_temp)
        echo -ne "\r${DIM}[${mode_name}] cooling to ${target}°C... ${current}°C (${elapsed}s)  ${RESET}"
        if (( current <= target )); then
            echo -e "\r${DIM}[${mode_name}] cooling to ${target}°C... ${current}°C ✓            ${RESET}"
            return 0
        fi
    done

    local final_temp
    final_temp=$(get_pkg_temp)
    echo -e "\r${YELLOW}[${mode_name}] cooldown timeout — ${final_temp}°C (target was ${target}°C)${RESET}"
    return 0  # continue anyway
}

# ─── CSV helpers ─────────────────────────────────────────────────────────────

write_csv_header() {
    echo "timestamp,mode,elapsed_s,pkg_temp,max_core_temp,avg_core_temp,avg_freq_mhz,power_watts,fan_rpm_z1,fan_rpm_z2,fan_rpm_z3,fan_rpm_z4" > "$CSV_FILE"
}

append_csv_row() {
    echo "$1" >> "$CSV_FILE"
}

# ─── Summary table ───────────────────────────────────────────────────────────

declare -A SUMMARY_IDLE SUMMARY_AVG SUMMARY_MAX SUMMARY_FREQ SUMMARY_POWER SUMMARY_MAXCORE

print_summary_table() {
    echo ""
    echo -e "${BOLD}Mode          Idle°C  Avg°C  Max°C  AvgFreq   AvgPower  MaxCore°C${RESET}"
    echo -e "${DIM}──────────────────────────────────────────────────────────────────────${RESET}"
    for mode in "${MODES[@]}"; do
        printf "%-14s %4s    %4s   %4s   %5sMHz   %5sW      %4s\n" \
            "$mode" \
            "${SUMMARY_IDLE[$mode]:-—}" \
            "${SUMMARY_AVG[$mode]:-—}" \
            "${SUMMARY_MAX[$mode]:-—}" \
            "${SUMMARY_FREQ[$mode]:-—}" \
            "${SUMMARY_POWER[$mode]:-—}" \
            "${SUMMARY_MAXCORE[$mode]:-—}"
    done
}

# ─── Main ────────────────────────────────────────────────────────────────────

main() {
    mkdir -p "$RESULTS_DIR"
    CSV_FILE="${RESULTS_DIR}/stress_$(date +%Y%m%d_%H%M%S).csv"
    write_csv_header

    echo -e "${BOLD}CPU Stress Test — ${NCPU} cores, ${DURATION}s per mode${RESET}"
    echo -e "${DIM}$(printf '─%.0s' {1..50})${RESET}"
    echo -e "${DIM}Results: ${CSV_FILE}${RESET}"
    echo ""

    for mode in "${MODES[@]}"; do
        # ── Cooldown ──
        # Switch to balanced first for lower idle power
        $BHELPER set perf balanced &>/dev/null || true
        cooldown "$COOLDOWN_TARGET" "$COOLDOWN_MAX_WAIT" "$mode"

        # ── Switch to custom + set CPU mode ──
        $BHELPER set perf custom &>/dev/null || fail "Failed to set custom perf mode"
        $BHELPER set cpu "$mode" &>/dev/null || fail "Failed to set CPU boost to $mode"
        # Set fans to manual at moderate speed during test (let thermal throttle naturally)
        $BHELPER set fan manual 3500 &>/dev/null || true
        sleep 1  # let mode settle

        # ── Record idle baseline ──
        local idle_sum=0
        for (( i = 0; i < IDLE_SAMPLES; i++ )); do
            local t
            t=$(get_pkg_temp)
            (( idle_sum += t ))
            sleep 1
        done
        local idle_temp=$(( idle_sum / IDLE_SAMPLES ))
        SUMMARY_IDLE[$mode]=$idle_temp

        # ── Start stress-ng ──
        stress-ng --matrix "$NCPU" --timeout "${DURATION}s" &>/dev/null &
        STRESS_PID=$!

        local start_time elapsed=0
        local max_temp=0 temp_sum=0 freq_sum=0 power_sum=0 sample_count=0
        local max_core_overall=0
        local rapl_prev rapl_prev_time

        start_time=$(date +%s%N)  # nanoseconds
        rapl_prev=$(get_rapl_energy_uj)
        rapl_prev_time=$start_time

        echo -e "${CYAN}[${mode}]${RESET} stressing..."

        # ── Sample loop ──
        while kill -0 "$STRESS_PID" 2>/dev/null; do
            sleep "$SAMPLE_INTERVAL"

            local now pkg_temp max_core avg_core avg_freq
            local rapl_now power_w fan_rpms

            now=$(date +%s%N)
            elapsed=$(( (now - start_time) / 1000000000 ))

            pkg_temp=$(get_pkg_temp)
            max_core=$(get_max_core_temp)
            avg_core=$(get_avg_core_temp)
            avg_freq=$(get_avg_freq_mhz)

            # Power from RAPL delta
            rapl_now=$(get_rapl_energy_uj)
            local elapsed_us=$(( (now - rapl_prev_time) / 1000 ))
            power_w=$(calc_power_watts "$rapl_prev" "$rapl_now" "$elapsed_us")
            rapl_prev=$rapl_now
            rapl_prev_time=$now

            fan_rpms=$(get_fan_rpms)

            # Append CSV
            local ts
            ts=$(date -Iseconds)
            append_csv_row "${ts},${mode},${elapsed},${pkg_temp},${max_core},${avg_core},${avg_freq},${power_w},${fan_rpms}"

            # Update running stats
            (( pkg_temp > max_temp )) && max_temp=$pkg_temp
            (( max_core > max_core_overall )) && max_core_overall=$max_core
            (( temp_sum += pkg_temp ))
            (( freq_sum += avg_freq ))
            # power_sum needs float math
            power_sum=$(python3 -c "print(f'{$power_sum + $power_w:.1f}')" 2>/dev/null || echo "$power_sum")
            (( sample_count++ ))

            # Live terminal line
            echo -ne "\r${CYAN}[${mode}]${RESET} ${elapsed}s  pkg:${pkg_temp}°C  freq:${avg_freq}MHz  power:${power_w}W  fans:${fan_rpms}   "
        done

        # Wait for stress-ng to finish
        wait "$STRESS_PID" 2>/dev/null || true
        STRESS_PID=""
        echo ""

        # ── Compute summary ──
        if (( sample_count > 0 )); then
            SUMMARY_AVG[$mode]=$(( temp_sum / sample_count ))
            SUMMARY_MAX[$mode]=$max_temp
            SUMMARY_FREQ[$mode]=$(( freq_sum / sample_count ))
            SUMMARY_POWER[$mode]=$(python3 -c "print(f'{$power_sum / $sample_count:.0f}')" 2>/dev/null || echo "—")
            SUMMARY_MAXCORE[$mode]=$max_core_overall
        fi
    done

    # ── Final output ──
    print_summary_table
    echo ""
    echo -e "${DIM}CSV saved to: ${CSV_FILE}${RESET}"
}

main
