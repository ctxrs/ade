import "@testing-library/jest-dom";

// Some UI dependencies (notably xterm) probe canvas APIs at import time.
// JSDOM doesn't implement canvas, so provide a small stub to keep unit tests
// focused on app behavior.
if (typeof HTMLCanvasElement !== "undefined") {
  // eslint-disable-next-line no-extend-native
  (HTMLCanvasElement.prototype as any).getContext ??= () => {
    return {
      fillStyle: "",
      fillRect: () => {},
      clearRect: () => {},
      getImageData: () => ({ data: new Uint8ClampedArray([0, 0, 0, 255]) }),
      putImageData: () => {},
      measureText: () => ({ width: 0 }),
    };
  };
}
