use peasy_core::{EngineDecision, EngineInput, ModelAction};

#[unsafe(no_mangle)]
pub extern "C" fn peasy_alloc(length: u32) -> u32 {
    let mut bytes = vec![0_u8; length as usize].into_boxed_slice();
    let pointer = bytes.as_mut_ptr();
    std::mem::forget(bytes);
    pointer as u32
}

#[unsafe(no_mangle)]
/// Releases a guest allocation previously returned by `peasy_alloc` or
/// `peasy_resolve`.
///
/// # Safety
///
/// `pointer` and `length` must identify exactly one live allocation returned
/// by those exports, and the allocation must not have been released already.
pub unsafe extern "C" fn peasy_dealloc(pointer: u32, length: u32) {
    if pointer != 0 {
        // SAFETY: The host returns only allocations made by peasy_alloc/resolve.
        let slice = std::ptr::slice_from_raw_parts_mut(pointer as *mut u8, length as usize);
        unsafe { drop(Box::from_raw(slice)) };
    }
}

fn decide(input: EngineInput) -> EngineDecision {
    match input.action {
        ModelAction::SearchPackage { query, version } => EngineDecision::Search { query, version },
        ModelAction::SearchAppImage {
            query,
            version,
            repository,
        } => EngineDecision::SearchAppImage {
            query,
            version,
            repository,
        },
        ModelAction::CheckPackage { query } => EngineDecision::CheckPackage(query),
        ModelAction::ListThemes => EngineDecision::ListThemes,
        ModelAction::ListWifi => EngineDecision::ListWifi,
        ModelAction::HyprlandStatus => EngineDecision::HyprlandStatus,
        ModelAction::InstallPackage { package, message } => {
            if input
                .candidates
                .iter()
                .any(|item| item.attribute == package)
            {
                EngineDecision::Install { package, message }
            } else {
                EngineDecision::Reject("model selected a package outside the candidate set".into())
            }
        }
        ModelAction::RemovePackage { package } => {
            if input.installed.iter().any(|item| item == &package) {
                EngineDecision::Remove(package)
            } else {
                EngineDecision::Reject("model selected a package Peasy does not manage".into())
            }
        }
        ModelAction::SetTheme { theme } => EngineDecision::SetTheme(theme),
        ModelAction::SetHyprlandSetting { change } => EngineDecision::SetHyprlandSetting(change),
        ModelAction::HyprlandDispatch { dispatch, argument } => {
            EngineDecision::HyprlandDispatch { dispatch, argument }
        }
        ModelAction::ConnectWifi { ssid } => EngineDecision::ConnectWifi(ssid),
        ModelAction::ConnectBluetooth { device } => EngineDecision::ConnectBluetooth(device),
        ModelAction::CreateCalendarEvent {
            title,
            start_local,
            duration_minutes,
        } => EngineDecision::CreateCalendarEvent {
            title,
            start_local,
            duration_minutes,
        },
        ModelAction::Explain { message } => EngineDecision::Explain(message),
        ModelAction::Cancel => EngineDecision::Cancel,
    }
}

#[unsafe(no_mangle)]
/// Resolves a serialized, typed engine request.
///
/// # Safety
///
/// `pointer..pointer + length` must be a live, initialized guest-memory range
/// allocated by `peasy_alloc`. The host must not mutate it during this call.
pub unsafe extern "C" fn peasy_resolve(pointer: u32, length: u32) -> u64 {
    // SAFETY: Wasmtime validates that the range is within the guest memory before calling.
    let input = unsafe { std::slice::from_raw_parts(pointer as *const u8, length as usize) };
    let decision = serde_json::from_slice::<EngineInput>(input)
        .map(decide)
        .unwrap_or_else(|_| EngineDecision::Reject("invalid typed engine input".into()));
    let mut output = serde_json::to_vec(&decision)
        .expect("serializing a fixed engine decision")
        .into_boxed_slice();
    let length = output.len() as u32;
    let pointer = output.as_mut_ptr() as u32;
    std::mem::forget(output);
    ((pointer as u64) << 32) | length as u64
}
