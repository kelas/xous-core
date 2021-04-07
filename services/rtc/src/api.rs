use xous::{Message, ScalarMessage};

#[derive(Debug, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub enum Weekday {
    Sunday,
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
}

#[derive(Debug, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub enum TimeUnits {
    Seconds,
    Minutes,
    Hours,
}

#[derive(Debug, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct DateTime {
    pub seconds: u8,
    pub minutes: u8,
    pub hours: u8,
    pub days: u8,
    pub months: u8,
    pub years: u8,
    pub weekday: Weekday,
}

pub(crate) const SERVER_NAME_RTC: &str       = "_Real time clock application server_";

#[derive(Debug, num_derive::FromPrimitive, num_derive::ToPrimitive)]
pub enum Opcode {
    /// register a callback for the datetime
    RegisterDateTimeCallback,

    /// register a callback for the RTC alarm event
    RegisterAlarmCallback,

    /// Get datetime. Causes users with registered callbacks to receive the current DateTime
    RequestDateTime,

    /// sets the datetime
    SetDateTime, //(DateTime),

    /// sets a wake-up alarm. This forces the SoC into power-on state, if it happens to be off.
    /// primarily used to trigger cold reboots, but could have other reasons
    SetWakeupAlarm, //(u8, TimeUnits),

    /// clear any wakeup alarms that have been set
    ClearWakeupAlarm,

    /// sets an RTC alarm. This just triggers a regular interrupt, no other side-effect
    SetRtcAlarm,

    /// clears any RTC alarms that have been set
    ClearRtcAlarm,
}

#[derive(Debug, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub(crate) enum Return {
    ReturnDateTime(DateTime),
    Drop,
}

#[derive(Debug, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Copy, Clone)]
pub(crate) struct ScalarHook {
    pub sid: (u32, u32, u32, u32),
    pub id: u32,  // ID of the scalar message to send through (e.g. the discriminant of the Enum on the caller's side API)
    pub cid: xous::CID,   // caller-side connection ID for the scalar message to route to. Created by the caller before hooking.
}
