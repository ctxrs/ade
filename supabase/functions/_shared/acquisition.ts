export type AcquisitionContext = {
  referrerDomain?: string | null;
  utmSource?: string | null;
  utmMedium?: string | null;
  utmCampaign?: string | null;
};

export type DownloadAttributionContext = AcquisitionContext & {
  downloadId?: string | null;
};

const ABSOLUTE_URL_PATTERN = /^[a-zA-Z][a-zA-Z\d+\-.]*:/;
const MAX_ATTRIBUTION_VALUE_LENGTH = 120;
const MAX_HOST_LENGTH = 253;

function normalizeAttributionValue(raw: string | null): string | null {
  if (!raw) return null;
  const trimmed = raw.trim();
  if (!trimmed) return null;
  return trimmed.slice(0, MAX_ATTRIBUTION_VALUE_LENGTH);
}

export function normalizeReferrerDomain(raw: string | null): string | null {
  if (!raw) return null;
  const trimmed = raw.trim();
  if (!trimmed) return null;
  try {
    const parsed = ABSOLUTE_URL_PATTERN.test(trimmed)
      ? new URL(trimmed)
      : new URL(`https://${trimmed}`);
    const hostname = parsed.hostname.trim().toLowerCase().replace(/\.+$/, "");
    if (!hostname || hostname.length > MAX_HOST_LENGTH) return null;
    return hostname;
  } catch {
    return null;
  }
}

export function readAcquisitionContext(req: Request, url: URL): AcquisitionContext {
  return {
    referrerDomain: normalizeReferrerDomain(url.searchParams.get("referrer_domain"))
      ?? normalizeReferrerDomain(req.headers.get("referer")),
    utmSource: normalizeAttributionValue(url.searchParams.get("utm_source")),
    utmMedium: normalizeAttributionValue(url.searchParams.get("utm_medium")),
    utmCampaign: normalizeAttributionValue(url.searchParams.get("utm_campaign")),
  };
}

export function attributionProperties(
  context: AcquisitionContext,
): Record<string, string> {
  return {
    ...(context.referrerDomain ? { referrer_domain: context.referrerDomain } : {}),
    ...(context.utmSource ? { utm_source: context.utmSource } : {}),
    ...(context.utmMedium ? { utm_medium: context.utmMedium } : {}),
    ...(context.utmCampaign ? { utm_campaign: context.utmCampaign } : {}),
  };
}

export function hasAttributionContext(context: DownloadAttributionContext): boolean {
  return Boolean(
    context.downloadId
      || context.referrerDomain
      || context.utmSource
      || context.utmMedium
      || context.utmCampaign,
  );
}

export function appendAttributionParamsToUrl(
  urlValue: string,
  context: DownloadAttributionContext,
): string {
  if (!hasAttributionContext(context)) return urlValue;
  try {
    const parsed = new URL(urlValue, "https://ctx.invalid");
    if (context.downloadId) parsed.searchParams.set("ctx_download_id", context.downloadId);
    if (context.referrerDomain) parsed.searchParams.set("referrer_domain", context.referrerDomain);
    if (context.utmSource) parsed.searchParams.set("utm_source", context.utmSource);
    if (context.utmMedium) parsed.searchParams.set("utm_medium", context.utmMedium);
    if (context.utmCampaign) parsed.searchParams.set("utm_campaign", context.utmCampaign);
    if (ABSOLUTE_URL_PATTERN.test(urlValue)) {
      return parsed.toString();
    }
    return `${parsed.pathname}${parsed.search}${parsed.hash}`;
  } catch {
    return urlValue;
  }
}
