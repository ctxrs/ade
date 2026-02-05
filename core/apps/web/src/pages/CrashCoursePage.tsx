import { Link } from "react-router-dom";
import LauncherBrand from "../components/LauncherBrand";

export default function CrashCoursePage() {
  return (
    <div className="crash-course-shell">
      <div className="crash-course-panel">
        <div className="launcher-header">
          <LauncherBrand />
          <div>
            <div className="crash-course-title">Crash course</div>
            <div className="crash-course-subtitle">5-minute walkthrough of the ctx workflow.</div>
          </div>
        </div>

        <video className="crash-course-video" controls preload="metadata">
          Your browser does not support the video tag.
        </video>

        <div className="crash-course-actions">
          <button type="button" className="crash-course-skip">Watch later</button>
          <Link to="/" className="wizard-primary">Continue to launcher</Link>
        </div>
      </div>
    </div>
  );
}
