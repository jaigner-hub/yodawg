import { useEffect, useRef, useState } from "react";
import RFB from "@novnc/novnc";

type Status = "connecting" | "connected" | "disconnected";

/**
 * Embeds a noVNC client that connects to QEMU's built-in VNC websocket
 * (`-vnc ...,websocket=PORT`). The VM's display renders into the canvas noVNC
 * creates inside our container div.
 */
export function VncViewer({ port }: { port: number }) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [status, setStatus] = useState<Status>("connecting");

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;

    setStatus("connecting");
    const rfb = new RFB(el, `ws://127.0.0.1:${port}`);
    rfb.scaleViewport = true; // fit the framebuffer to our panel
    rfb.focusOnClick = true;

    const onConnect = () => setStatus("connected");
    const onDisconnect = () => setStatus("disconnected");
    rfb.addEventListener("connect", onConnect);
    rfb.addEventListener("disconnect", onDisconnect);

    return () => {
      rfb.removeEventListener("connect", onConnect);
      rfb.removeEventListener("disconnect", onDisconnect);
      try {
        rfb.disconnect();
      } catch {
        /* already gone */
      }
    };
  }, [port]);

  return (
    <div className="vnc-wrap">
      <div ref={containerRef} className="vnc-canvas" />
      {status !== "connected" && (
        <div className="vnc-overlay">
          {status === "connecting"
            ? "Connecting to display…"
            : "Display disconnected"}
        </div>
      )}
    </div>
  );
}
