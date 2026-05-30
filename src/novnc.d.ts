// Minimal type declarations for noVNC (the package ships no .d.ts).
declare module "@novnc/novnc" {
  export interface RFBOptions {
    credentials?: { username?: string; password?: string; target?: string };
    shared?: boolean;
    repeaterID?: string;
    wsProtocols?: string[];
  }

  export default class RFB extends EventTarget {
    constructor(
      target: HTMLElement,
      url: string | WebSocket | RTCDataChannel,
      options?: RFBOptions
    );
    /** Scale the remote framebuffer to fit the container. */
    scaleViewport: boolean;
    /** Ask the server to resize to match the container (needs server support). */
    resizeSession: boolean;
    /** Whether to grab keyboard/pointer focus. */
    focusOnClick: boolean;
    viewOnly: boolean;
    disconnect(): void;
    focus(): void;
    sendCtrlAltDel(): void;
  }
}
