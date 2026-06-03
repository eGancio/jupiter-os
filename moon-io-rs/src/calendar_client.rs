// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

use chrono::{DateTime, NaiveDate, Utc};
use reqwest::Client;
use tracing::info;
use uuid::Uuid;

use crate::error::{IoError, Result};

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct CalendarEvent {
    pub uid: String,
    pub summary: String,
    pub start: String,
    pub end: String,
    pub description: Option<String>,
    pub location: Option<String>,
    pub organizer: Option<String>,
    pub attendees: Vec<String>,
    pub status: String,
    pub url: Option<String>,
}

pub struct CreateEventParams {
    pub summary: String,
    pub start: String,
    pub end: String,
    pub description: Option<String>,
    pub location: Option<String>,
    pub attendees: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
pub enum EventResponse {
    Accepted,
    Declined,
    Tentative,
}

impl EventResponse {
    pub fn as_partstat(&self) -> &str {
        match self {
            Self::Accepted => "ACCEPTED",
            Self::Declined => "DECLINED",
            Self::Tentative => "TENTATIVE",
        }
    }
}

// ---------------------------------------------------------------------------
// CalendarClient — CalDAV via HTTP + XML
// ---------------------------------------------------------------------------

pub struct CalendarClient {
    url: String,
    username: String,
    password: String,
    http: Client,
}

impl CalendarClient {
    pub fn new(url: &str, username: &str, password: &str) -> Self {
        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("Failed to build HTTP client");

        Self {
            url: url.trim_end_matches('/').to_string(),
            username: username.to_string(),
            password: password.to_string(),
            http,
        }
    }

    /// Test connection to CalDAV server.
    pub async fn test_connection(&self) -> Result<()> {
        let resp = self
            .http
            .request(reqwest::Method::from_bytes(b"PROPFIND").unwrap(), &self.url)
            .basic_auth(&self.username, Some(&self.password))
            .header("Depth", "0")
            .header("Content-Type", "application/xml; charset=utf-8")
            .body(r#"<?xml version="1.0" encoding="UTF-8"?>
<d:propfind xmlns:d="DAV:">
  <d:prop><d:displayname/></d:prop>
</d:propfind>"#)
            .send()
            .await
            .map_err(|e| IoError::CalDav(format!("PROPFIND: {e}")))?;

        if resp.status().is_success() || resp.status().as_u16() == 207 {
            info!("CalDAV connection OK: {}", self.url);
            Ok(())
        } else {
            Err(IoError::CalDav(format!(
                "CalDAV returned {}: {}",
                resp.status(),
                resp.text().await.unwrap_or_default()
            )))
        }
    }

    /// Create a calendar event. Returns the event UID.
    pub async fn create_event(&self, params: CreateEventParams) -> Result<String> {
        let uid = Uuid::new_v4().to_string();
        let now = Utc::now().format("%Y%m%dT%H%M%SZ");

        let mut vcal = format!(
            "BEGIN:VCALENDAR\r\n\
             VERSION:2.0\r\n\
             PRODID:-//MCP Moon Io//EN\r\n\
             BEGIN:VEVENT\r\n\
             UID:{uid}\r\n\
             DTSTAMP:{now}\r\n\
             DTSTART:{}\r\n\
             DTEND:{}\r\n\
             SUMMARY:{}\r\n",
            format_ical_date(&params.start),
            format_ical_date(&params.end),
            escape_ical(&params.summary),
        );

        if let Some(ref desc) = params.description {
            vcal.push_str(&format!("DESCRIPTION:{}\r\n", escape_ical(desc)));
        }
        if let Some(ref loc) = params.location {
            vcal.push_str(&format!("LOCATION:{}\r\n", escape_ical(loc)));
        }

        // Organizer + attendees
        if !params.attendees.is_empty() {
            vcal.push_str(&format!(
                "ORGANIZER;CN={}:MAILTO:{}\r\n",
                self.username, self.username
            ));
            for attendee in &params.attendees {
                vcal.push_str(&format!(
                    "ATTENDEE;ROLE=REQ-PARTICIPANT;PARTSTAT=NEEDS-ACTION;RSVP=TRUE:MAILTO:{}\r\n",
                    attendee
                ));
            }
        }

        vcal.push_str("END:VEVENT\r\nEND:VCALENDAR\r\n");

        let event_url = format!("{}/{}.ics", self.url, uid);

        let resp = self
            .http
            .put(&event_url)
            .basic_auth(&self.username, Some(&self.password))
            .header("Content-Type", "text/calendar; charset=utf-8")
            .body(vcal)
            .send()
            .await
            .map_err(|e| IoError::CalDav(format!("PUT event: {e}")))?;

        if resp.status().is_success() || resp.status().as_u16() == 201 {
            info!("Created event: {uid}");
            Ok(uid)
        } else {
            Err(IoError::CalDav(format!(
                "Create event failed ({}): {}",
                resp.status(),
                resp.text().await.unwrap_or_default()
            )))
        }
    }

    /// List calendar events in a date range.
    pub async fn list_events(
        &self,
        start_date: Option<&str>,
        end_date: Option<&str>,
        limit: usize,
    ) -> Result<Vec<CalendarEvent>> {
        let start = start_date
            .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
            .unwrap_or_else(|| Utc::now().date_naive());
        let end = end_date
            .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
            .unwrap_or_else(|| start + chrono::Duration::days(30));

        let start_ical = start.format("%Y%m%dT000000Z");
        let end_ical = end.format("%Y%m%dT235959Z");

        let report_xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<c:calendar-query xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
  <d:prop>
    <d:getetag/>
    <c:calendar-data/>
  </d:prop>
  <c:filter>
    <c:comp-filter name="VCALENDAR">
      <c:comp-filter name="VEVENT">
        <c:time-range start="{start_ical}" end="{end_ical}"/>
      </c:comp-filter>
    </c:comp-filter>
  </c:filter>
</c:calendar-query>"#
        );

        let resp = self
            .http
            .request(reqwest::Method::from_bytes(b"REPORT").unwrap(), &self.url)
            .basic_auth(&self.username, Some(&self.password))
            .header("Depth", "1")
            .header("Content-Type", "application/xml; charset=utf-8")
            .body(report_xml)
            .send()
            .await
            .map_err(|e| IoError::CalDav(format!("REPORT: {e}")))?;

        let body = resp
            .text()
            .await
            .map_err(|e| IoError::CalDav(format!("Read body: {e}")))?;

        let mut events = parse_caldav_response(&body);
        events.sort_by(|a, b| a.start.cmp(&b.start));
        events.truncate(limit);

        Ok(events)
    }

    /// Respond to a calendar event (accept/decline/tentative).
    pub async fn respond_to_event(
        &self,
        event_uid: &str,
        response: EventResponse,
        comment: Option<&str>,
    ) -> Result<()> {
        // First, find the event
        let event_url = format!("{}/{}.ics", self.url, event_uid);

        let resp = self
            .http
            .get(&event_url)
            .basic_auth(&self.username, Some(&self.password))
            .send()
            .await
            .map_err(|e| IoError::CalDav(format!("GET event: {e}")))?;

        if !resp.status().is_success() {
            return Err(IoError::NotFound(format!(
                "Event {event_uid} not found"
            )));
        }

        let ical_text = resp
            .text()
            .await
            .map_err(|e| IoError::CalDav(format!("Read event: {e}")))?;

        // Update PARTSTAT for our attendee
        let mailto = format!("MAILTO:{}", self.username);
        let partstat = response.as_partstat();

        // Simple string replacement for PARTSTAT
        // Look for ATTENDEE line containing our email and update PARTSTAT
        let new_ical = update_partstat(&ical_text, &mailto, partstat, comment);

        let resp = self
            .http
            .put(&event_url)
            .basic_auth(&self.username, Some(&self.password))
            .header("Content-Type", "text/calendar; charset=utf-8")
            .body(new_ical)
            .send()
            .await
            .map_err(|e| IoError::CalDav(format!("PUT event: {e}")))?;

        if resp.status().is_success() {
            info!("Responded {partstat} to event {event_uid}");
            Ok(())
        } else {
            Err(IoError::CalDav(format!(
                "Update event failed ({}): {}",
                resp.status(),
                resp.text().await.unwrap_or_default()
            )))
        }
    }

    /// Delete a calendar event.
    pub async fn delete_event(&self, event_uid: &str) -> Result<()> {
        let event_url = format!("{}/{}.ics", self.url, event_uid);

        let resp = self
            .http
            .delete(&event_url)
            .basic_auth(&self.username, Some(&self.password))
            .send()
            .await
            .map_err(|e| IoError::CalDav(format!("DELETE: {e}")))?;

        if resp.status().is_success() || resp.status().as_u16() == 204 {
            info!("Deleted event {event_uid}");
            Ok(())
        } else {
            Err(IoError::CalDav(format!(
                "Delete failed ({}): {}",
                resp.status(),
                resp.text().await.unwrap_or_default()
            )))
        }
    }
}

// ---------------------------------------------------------------------------
// iCalendar parsing helpers
// ---------------------------------------------------------------------------

/// Parse a CalDAV REPORT multistatus response and extract VEVENT data.
fn parse_caldav_response(xml: &str) -> Vec<CalendarEvent> {
    let mut events = Vec::new();

    // Extract <cal:calendar-data> blocks containing VCALENDAR
    // Simple regex-based extraction (CalDAV XML is well-structured)
    let cal_re = regex::Regex::new(r"(?s)BEGIN:VCALENDAR.*?END:VCALENDAR").unwrap();

    for cal_match in cal_re.find_iter(xml) {
        let ical = cal_match.as_str();
        if let Some(event) = parse_vevent(ical) {
            events.push(event);
        }
    }

    events
}

/// Parse a VCALENDAR string and extract the first VEVENT.
fn parse_vevent(ical: &str) -> Option<CalendarEvent> {
    let vevent_re = regex::Regex::new(r"(?s)BEGIN:VEVENT(.*?)END:VEVENT").unwrap();
    let caps = vevent_re.captures(ical)?;
    let vevent = caps.get(1)?.as_str();

    let uid = extract_ical_prop(vevent, "UID")?;
    let summary = extract_ical_prop(vevent, "SUMMARY").unwrap_or_default();
    let start = extract_ical_prop(vevent, "DTSTART")
        .or_else(|| extract_ical_prop_with_params(vevent, "DTSTART"))
        .unwrap_or_default();
    let end = extract_ical_prop(vevent, "DTEND")
        .or_else(|| extract_ical_prop_with_params(vevent, "DTEND"))
        .unwrap_or_default();
    let description = extract_ical_prop(vevent, "DESCRIPTION");
    let location = extract_ical_prop(vevent, "LOCATION");
    let status = extract_ical_prop(vevent, "STATUS").unwrap_or_else(|| "CONFIRMED".into());
    let organizer = extract_ical_prop(vevent, "ORGANIZER");

    // Extract attendees
    let attendee_re = regex::Regex::new(r"ATTENDEE[^:]*:MAILTO:([^\r\n]+)").unwrap();
    let attendees: Vec<String> = attendee_re
        .captures_iter(vevent)
        .map(|c| c[1].to_string())
        .collect();

    Some(CalendarEvent {
        uid,
        summary,
        start,
        end,
        description,
        location,
        organizer,
        attendees,
        status,
        url: None,
    })
}

/// Extract a simple iCalendar property value.
fn extract_ical_prop(text: &str, prop: &str) -> Option<String> {
    let re = regex::Regex::new(&format!(r"(?m)^{}:(.+?)$", regex::escape(prop))).ok()?;
    re.captures(text)
        .map(|c| c[1].trim().to_string())
}

/// Extract an iCalendar property with parameters (e.g., DTSTART;VALUE=DATE:20260301).
fn extract_ical_prop_with_params(text: &str, prop: &str) -> Option<String> {
    let re = regex::Regex::new(&format!(r"(?m)^{};[^:]+:(.+?)$", regex::escape(prop))).ok()?;
    re.captures(text)
        .map(|c| c[1].trim().to_string())
}

/// Format a date string for iCalendar (YYYYMMDDTHHMMSSZ).
fn format_ical_date(date_str: &str) -> String {
    // Try parsing various formats
    if let Ok(dt) = DateTime::parse_from_rfc3339(date_str) {
        return dt.format("%Y%m%dT%H%M%SZ").to_string();
    }
    if let Ok(dt) = NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
        return dt.format("%Y%m%dT000000Z").to_string();
    }
    // Assume it's already in iCal format or pass through
    date_str.to_string()
}

/// Escape text for iCalendar (backslash-escape commas, semicolons, newlines).
fn escape_ical(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace(';', "\\;")
        .replace(',', "\\,")
        .replace('\n', "\\n")
}

/// Update PARTSTAT in a VCALENDAR for a specific attendee.
fn update_partstat(ical: &str, mailto: &str, partstat: &str, _comment: Option<&str>) -> String {
    let mut result = String::new();
    let mailto_lower = mailto.to_lowercase();

    for line in ical.lines() {
        if line.to_lowercase().contains(&mailto_lower) && line.starts_with("ATTENDEE") {
            // Replace PARTSTAT value
            let partstat_re = regex::Regex::new(r"PARTSTAT=[A-Z\-]+").unwrap();
            let updated = if partstat_re.is_match(line) {
                partstat_re
                    .replace(line, &format!("PARTSTAT={partstat}"))
                    .to_string()
            } else {
                // Add PARTSTAT if not present
                line.replacen("ATTENDEE", &format!("ATTENDEE;PARTSTAT={partstat}"), 1)
            };
            result.push_str(&updated);
        } else {
            result.push_str(line);
        }
        result.push_str("\r\n");
    }

    result
}
