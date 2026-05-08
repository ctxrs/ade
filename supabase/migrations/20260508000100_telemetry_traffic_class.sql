alter table public.telemetry_event
  add column if not exists analytics_environment text,
  add column if not exists traffic_class text;

alter table public.telemetry_event
  alter column traffic_class drop default,
  alter column traffic_class drop not null;

do $$
begin
  if not exists (
    select 1
    from pg_constraint
    where conname = 'telemetry_event_traffic_class_check'
      and conrelid = 'public.telemetry_event'::regclass
  ) then
    alter table public.telemetry_event
      add constraint telemetry_event_traffic_class_check
      check (
        traffic_class is null or
        traffic_class in ('user', 'synthetic', 'internal', 'load_test', 'ci')
      );
  end if;
end $$;

create index if not exists telemetry_event_analytics_environment_idx
  on public.telemetry_event (analytics_environment);

create index if not exists telemetry_event_traffic_class_idx
  on public.telemetry_event (traffic_class);

create index if not exists telemetry_event_real_desktop_idx
  on public.telemetry_event (
    analytics_environment,
    traffic_class,
    origin_runtime,
    surface,
    occurred_at desc
  );
