//! Browser backend (`wasm32-unknown-unknown`, DESIGN 2.1 / 5.4 / S12): no tzdb in the
//! binary. A named zone is an `Intl.DateTimeFormat` with that `timeZone`: its
//! `formatToParts` gives the wall time of an instant, the offset is wall − UTC. The local
//! zone without a name is `Date.prototype.getTimezoneOffset`.
//!
//! JS objects are not `Send`, so a `Zone` holds only an id; the formatters live per thread
//! (rebuilt from the name on a thread that has not seen the zone). Offsets are cached per
//! (zone, UTC hour) in a fixed table of 256 slots, so the cache never grows: an hour is
//! cached only when its first and last second have the same offset, which keeps instants
//! next to a transition at :30 or :45 exact.

use super::{Rules, UNKNOWN, Zone, days_from_civil};
use js_sys::wasm_bindgen::{JsCast, JsValue};
use js_sys::{Array, Date, Function, Intl, Object, Reflect};
use std::cell::RefCell;
use std::sync::atomic::{AtomicU32, Ordering};

/// `id` 0 is the local zone (`getTimezoneOffset`), others index the per-thread formatters.
#[derive(Debug)]
pub(crate) struct JsZone {
    id: u32,
}

/// JS `Date` covers ±8.64e15 ms.
const MAX_SECONDS: i64 = 8_640_000_000_000;
const SLOTS: usize = 256;

thread_local! {
    static FORMATS: RefCell<Vec<(u32, Intl::DateTimeFormat)>> = const { RefCell::new(Vec::new()) };
    static OFFSETS: RefCell<[(u32, i64, i32); SLOTS]> = const { RefCell::new([(0, 0, 0); SLOTS]) };
}

fn get(o: &JsValue, key: &str) -> Option<JsValue> {
    Reflect::get(o, &JsValue::from_str(key)).ok()
}

/// `new Intl.DateTimeFormat("en-US", { timeZone, … })`, `None` when it throws (unknown
/// zone). Built through `Reflect.construct`, which catches the `RangeError`.
fn formatter(time_zone: Option<&str>) -> Option<Intl::DateTimeFormat> {
    let ctor: Function = get(&get(&js_sys::global(), "Intl")?, "DateTimeFormat")?
        .dyn_into()
        .ok()?;
    let opts = Object::new();
    if let Some(tz) = time_zone {
        let fields = [
            ("timeZone", tz),
            ("hourCycle", "h23"),
            ("calendar", "gregory"),
            ("numberingSystem", "latn"),
            ("era", "short"),
            ("year", "numeric"),
            ("month", "numeric"),
            ("day", "numeric"),
            ("hour", "numeric"),
            ("minute", "numeric"),
            ("second", "numeric"),
        ];
        for (k, v) in fields {
            Reflect::set(&opts, &JsValue::from_str(k), &JsValue::from_str(v)).ok()?;
        }
    }
    let args = Array::new();
    args.push(&JsValue::from_str("en-US"));
    args.push(&opts);
    Some(Reflect::construct(&ctor, &args).ok()?.unchecked_into())
}

/// Wall time − UTC at `t` from `formatToParts` (era `BC` → astronomical year).
/// `None` for an instant outside `Date` (`formatToParts` would throw a `RangeError`).
fn intl_offset(fmt: &Intl::DateTimeFormat, t: i64) -> Option<i32> {
    let date = Date::new(&JsValue::from_f64((t * 1000) as f64));
    if date.get_time().is_nan() {
        return None;
    }
    let (mut y, mut mo, mut d, mut hms, mut bc) = (0i64, 1u8, 1u8, 0i64, false);
    for part in fmt.format_to_parts(&date).iter() {
        let ty = get(&part, "type")?.as_string()?;
        let v = get(&part, "value")?.as_string()?;
        match ty.as_str() {
            "era" => bc = v.starts_with('B'),
            "year" => y = v.parse().ok()?,
            "month" => mo = v.parse().ok()?,
            "day" => d = v.parse().ok()?,
            "hour" => hms += v.parse::<i64>().ok()? * 3600,
            "minute" => hms += v.parse::<i64>().ok()? * 60,
            "second" => hms += v.parse::<i64>().ok()?,
            _ => {}
        }
    }
    if bc {
        y = 1 - y;
    }
    i32::try_from(days_from_civil(y, mo, d) * 86_400 + hms - t).ok()
}

/// The formatter of zone `id`, built from `name` on a thread that has not seen it. A clone
/// (one JS reference) comes out, so no JS call runs while `FORMATS` is borrowed: an
/// exception thrown through wasm runs no destructor and would leave the borrow taken.
fn formatter_of(id: u32, name: &str) -> Option<Intl::DateTimeFormat> {
    if let Some(fmt) =
        FORMATS.with_borrow(|list| list.iter().find(|(i, _)| *i == id).map(|(_, f)| f.clone()))
    {
        return Some(fmt);
    }
    let fmt = formatter(Some(name))?;
    FORMATS.with_borrow_mut(|list| list.push((id, fmt.clone())));
    Some(fmt)
}

impl JsZone {
    pub(crate) fn offset_at(&self, name: &str, t: i64) -> i32 {
        let t = t.clamp(-MAX_SECONDS, MAX_SECONDS);
        if self.id == 0 {
            let minutes = Date::new(&JsValue::from_f64((t * 1000) as f64)).get_timezone_offset();
            return (-minutes * 60.0).round() as i32;
        }
        let hour = t.div_euclid(3600);
        let slot =
            (hour as u64 ^ u64::from(self.id).wrapping_mul(0x9e37_79b9_7f4a_7c15)) as usize % SLOTS;
        if let (id, h, off) = OFFSETS.with_borrow(|c| c[slot])
            && id == self.id
            && h == hour
        {
            return off;
        }
        let Some(fmt) = formatter_of(self.id, name) else {
            return 0;
        };
        // the last hour of `Date` ends at its last second: no instant past it is asked
        let first = intl_offset(&fmt, hour * 3600);
        if first.is_some() && first == intl_offset(&fmt, (hour * 3600 + 3599).min(MAX_SECONDS)) {
            let off = first.unwrap_or(0);
            OFFSETS.with_borrow_mut(|c| c[slot] = (self.id, hour, off));
            off
        } else {
            intl_offset(&fmt, t).unwrap_or(0)
        }
    }
}

/// The name `Intl` resolves `name` to (`europe/berlin` → `Europe/Berlin`), `None` if it
/// throws on it: the key of the zone cache, so spellings of one zone share one entry.
pub(crate) fn canonical(name: &str) -> Option<String> {
    get(&formatter(Some(name))?.resolved_options(), "timeZone")?.as_string()
}

/// The zone `name` if this engine's `Intl` knows it.
pub(crate) fn named(name: &str) -> Option<Zone> {
    static NEXT: AtomicU32 = AtomicU32::new(1);
    let fmt = formatter(Some(name))?;
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    FORMATS.with_borrow_mut(|list| list.push((id, fmt)));
    Some(Zone::new(name, Rules::Js(JsZone { id })))
}

/// `Intl.DateTimeFormat().resolvedOptions().timeZone`, else the local `Date` offsets.
pub(crate) fn system() -> Zone {
    formatter(None)
        .and_then(|f| get(&f.resolved_options(), "timeZone")?.as_string())
        .and_then(|tz| Zone::named(&tz))
        .unwrap_or_else(|| Zone::new(UNKNOWN, Rules::Js(JsZone { id: 0 })))
}
