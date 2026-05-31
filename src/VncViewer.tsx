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
  const rfbRef = useRef<RFB | null>(null);
  const [status, setStatus] = useState<Status>("connecting");
  // "fit" scales the framebuffer to the panel; "actual" renders it 1:1.
  //
  // 1:1 matters for guests that only have a *relative* pointing device — DOS
  // and Windows 3.1 predate USB, so they can't use our usb-tablet absolute
  // pointer and fall back to a relative PS/2 mouse. When the image is scaled,
  // one host pixel of motion no longer equals one guest pixel, so QEMU's
  // host-position-to-relative-delta conversion drifts and the offset grows
  // until the guest cursor can't reach the target. At 1:1 the deltas map
  // correctly and tracking holds. Modern guests use the absolute tablet, so
  // scaling is harmless for them — hence a toggle rather than forcing a mode.
  const [zoom, setZoom] = useState<"fit" | "actual">("fit");

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;

    setStatus("connecting");
    const rfb = new RFB(el, `ws://127.0.0.1:${port}`);
    rfbRef.current = rfb;
    rfb.scaleViewport = zoom === "fit";
    rfb.focusOnClick = true;

    const onConnect = () => setStatus("connected");
    const onDisconnect = () => setStatus("disconnected");
    rfb.addEventListener("connect", onConnect);
    rfb.addEventListener("disconnect", onDisconnect);

    return () => {
      rfb.removeEventListener("connect", onConnect);
      rfb.removeEventListener("disconnect", onDisconnect);
      rfbRef.current = null;
      try {
        rfb.disconnect();
      } catch {
        /* already gone */
      }
    };
    // Only rebuild the connection when the port changes; zoom is applied live
    // by the effect below without reconnecting.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [port]);

  // Apply zoom changes to the existing connection (no reconnect). At 1:1 the
  // canvas renders full framebuffer size; CSS `overflow:auto` on the container
  // lets it scroll rather than overflow the layout.
  useEffect(() => {
    const rfb = rfbRef.current;
    if (!rfb) return;
    rfb.scaleViewport = zoom === "fit";
  }, [zoom]);

  return (
    <div className="vnc-wrap">
      <div ref={containerRef} className={`vnc-canvas ${zoom}`} />
      <button
        className="vnc-zoom-toggle"
        title={
          zoom === "fit"
            ? "Switch to 1:1 — fixes mouse drift for DOS / Windows 3.1"
            : "Switch to Fit — scale the display to the panel"
        }
        onClick={() => setZoom((z) => (z === "fit" ? "actual" : "fit"))}
      >
        {zoom === "fit" ? "1:1" : "Fit"}
      </button>
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
