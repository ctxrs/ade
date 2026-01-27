import { useCallback, useEffect, useState } from "react";
import { Link } from "react-router-dom";
import type { UpdateCheck } from "../api/client";
import { readCachedUpdateCheck, refreshUpdateCheck } from "../utils/updateNotice";

export default function UpdateNoticeBanner() {
  const [updateInfo, setUpdateInfo] = useState<UpdateCheck | null>(() => readCachedUpdateCheck());
  const [checking, setChecking] = useState(false);

  useEffect(() => {
    refreshUpdateCheck().then((info) => {
      if (info) setUpdateInfo(info);
    });
  }, []);

  const onCheck = useCallback(async () => {
    setChecking(true);
    const info = await refreshUpdateCheck({ force: true });
    if (info) setUpdateInfo(info);
    setChecking(false);
  }, []);

  if (!updateInfo?.update_available) return null;
  const latest = updateInfo.latest_version ?? "unknown";

  return (
    <div className="banner" style={{ display: "flex", alignItems: "center", gap: 8 }}>
      <div style={{ flex: 1 }}>
        Update available: {latest}. <Link to="/diagnostics">Open Diagnostics</Link> to download and apply.
      </div>
      <button type="button" onClick={onCheck} disabled={checking}>
        {checking ? "Checking..." : "Check again"}
      </button>
    </div>
  );
}
