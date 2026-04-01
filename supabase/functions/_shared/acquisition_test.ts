import { assertEquals } from "https://deno.land/std@0.224.0/assert/assert_equals.ts";
import {
  appendAttributionParamsToUrl,
  attributionProperties,
  normalizeReferrerDomain,
  readAcquisitionContext,
} from "./acquisition.ts";

Deno.test("normalizeReferrerDomain extracts a bounded hostname", () => {
  assertEquals(normalizeReferrerDomain("https://ctx.rs/install?x=1"), "ctx.rs");
  assertEquals(normalizeReferrerDomain("WWW.Example.COM"), "www.example.com");
  assertEquals(normalizeReferrerDomain(""), null);
  assertEquals(normalizeReferrerDomain("::::"), null);
});

Deno.test("readAcquisitionContext prefers explicit query params over referer headers", () => {
  const request = new Request("https://api.ctx.rs/functions/v1/download/stable/1.0.0/app", {
    headers: {
      referer: "https://ctx.rs/blog/public-beta",
    },
  });
  const url = new URL(
    "https://api.ctx.rs/functions/v1/download/stable/1.0.0/app?referrer_domain=launch.ctx.rs&utm_source=twitter&utm_medium=social&utm_campaign=public-beta",
  );
  assertEquals(readAcquisitionContext(request, url), {
    referrerDomain: "launch.ctx.rs",
    utmSource: "twitter",
    utmMedium: "social",
    utmCampaign: "public-beta",
  });
});

Deno.test("appendAttributionParamsToUrl appends download and acquisition params", () => {
  assertEquals(
    appendAttributionParamsToUrl("/download/stable/1.0.0/app", {
      downloadId: "dl_123",
      referrerDomain: "ctx.rs",
      utmSource: "twitter",
      utmMedium: "social",
      utmCampaign: "public-beta",
    }),
    "/download/stable/1.0.0/app?ctx_download_id=dl_123&referrer_domain=ctx.rs&utm_source=twitter&utm_medium=social&utm_campaign=public-beta",
  );
  assertEquals(
    attributionProperties({
      referrerDomain: "ctx.rs",
      utmSource: "twitter",
      utmMedium: "social",
      utmCampaign: "public-beta",
    }),
    {
      referrer_domain: "ctx.rs",
      utm_source: "twitter",
      utm_medium: "social",
      utm_campaign: "public-beta",
    },
  );
});
