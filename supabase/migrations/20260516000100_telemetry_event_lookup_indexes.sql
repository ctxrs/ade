create index if not exists telemetry_event_event_id_idx
  on public.telemetry_event (event_id)
  where event_id is not null;

create index if not exists telemetry_event_event_name_ts_idx
  on public.telemetry_event (event_name, ts desc);
