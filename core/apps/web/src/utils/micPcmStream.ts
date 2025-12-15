type MicPcmStream = {
  stop: () => Promise<void>;
};

type StartMicPcmStreamOpts = {
  onPcmChunk: (pcm16: ArrayBuffer) => void;
  onError?: (err: Error) => void;
  chunkSamples?: number;
};

const DEFAULT_TARGET_SAMPLE_RATE = 16000;

function downsampleBuffer(buffer: Float32Array, inSampleRate: number, outSampleRate: number): Float32Array {
  if (outSampleRate === inSampleRate) return buffer;
  if (outSampleRate > inSampleRate) throw new Error("Upsampling not supported");
  const sampleRateRatio = inSampleRate / outSampleRate;
  const newLength = Math.round(buffer.length / sampleRateRatio);
  const result = new Float32Array(newLength);
  let offsetResult = 0;
  let offsetBuffer = 0;
  while (offsetResult < result.length) {
    const nextOffsetBuffer = Math.round((offsetResult + 1) * sampleRateRatio);
    let accum = 0;
    let count = 0;
    for (let i = offsetBuffer; i < nextOffsetBuffer && i < buffer.length; i++) {
      accum += buffer[i] ?? 0;
      count++;
    }
    result[offsetResult] = count ? accum / count : 0;
    offsetResult++;
    offsetBuffer = nextOffsetBuffer;
  }
  return result;
}

function floatTo16BitPCM(float32: Float32Array): Int16Array {
  const out = new Int16Array(float32.length);
  for (let i = 0; i < float32.length; i++) {
    const s = Math.max(-1, Math.min(1, float32[i] ?? 0));
    out[i] = s < 0 ? Math.round(s * 0x8000) : Math.round(s * 0x7fff);
  }
  return out;
}

export async function startMicPcmStream(opts: StartMicPcmStreamOpts): Promise<MicPcmStream> {
  const chunkSamples = opts.chunkSamples ?? 320; // 20ms @ 16kHz

  const stream = await navigator.mediaDevices.getUserMedia({
    audio: {
      channelCount: 1,
      echoCancellation: true,
      noiseSuppression: true,
      autoGainControl: true,
    },
  });

  const audioContext = new AudioContext({ sampleRate: DEFAULT_TARGET_SAMPLE_RATE });
  const source = audioContext.createMediaStreamSource(stream);
  const sink = audioContext.createGain();
  sink.gain.value = 0;
  sink.connect(audioContext.destination);

  // Some browsers create the context in a suspended state until a user gesture occurs.
  // (This is called from a click handler, but we still resume defensively.)
  try {
    if (audioContext.state === "suspended") {
      await audioContext.resume();
    }
  } catch {}

  let disconnectGraph: (() => void) | null = null;
  let detachHandler: (() => void) | null = null;

  let pcmCarry = new Int16Array(0);
  let gotAnySamples = false;

  const onFloats = (input: Float32Array) => {
    try {
      const sr = audioContext.sampleRate;
      const down = downsampleBuffer(input, sr, DEFAULT_TARGET_SAMPLE_RATE);
      const pcm = floatTo16BitPCM(down);

      const combined = new Int16Array(pcmCarry.length + pcm.length);
      combined.set(pcmCarry, 0);
      combined.set(pcm, pcmCarry.length);

      let offset = 0;
      while (offset + chunkSamples <= combined.length) {
        const chunk = combined.slice(offset, offset + chunkSamples);
        opts.onPcmChunk(chunk.buffer);
        offset += chunkSamples;
      }
      pcmCarry = combined.slice(offset);
      gotAnySamples = true;
    } catch (e: any) {
      opts.onError?.(e instanceof Error ? e : new Error(String(e)));
    }
  };

  if (audioContext.audioWorklet?.addModule) {
    const workletCode = `
      class MicProcessor extends AudioWorkletProcessor {
        process(inputs) {
          const input = inputs[0];
          if (input && input[0] && input[0].length) {
            const copy = input[0].slice(0);
            this.port.postMessage(copy);
          }
          return true;
        }
      }
      registerProcessor('mic-processor', MicProcessor);
    `;
    const blob = new Blob([workletCode], { type: "application/javascript" });
    const url = URL.createObjectURL(blob);

    await audioContext.audioWorklet.addModule(url);
    URL.revokeObjectURL(url);

    const node = new AudioWorkletNode(audioContext, "mic-processor");
    source.connect(node);
    node.connect(sink);
    disconnectGraph = () => {
      try {
        node.disconnect();
      } catch {}
      try {
        source.disconnect();
      } catch {}
    };

    const onMessage = (ev: MessageEvent) => {
      const data = ev.data;
      if (data instanceof Float32Array) {
        onFloats(data);
      } else if (data instanceof ArrayBuffer) {
        onFloats(new Float32Array(data));
      }
    };
    node.port.addEventListener("message", onMessage);
    node.port.start();
    detachHandler = () => node.port.removeEventListener("message", onMessage);
  } else {
    // Fallback for browsers without AudioWorklet support.
    const processor = audioContext.createScriptProcessor(2048, 1, 1);
    processor.onaudioprocess = (e) => {
      const input = e.inputBuffer.getChannelData(0);
      onFloats(new Float32Array(input));
    };
    source.connect(processor);
    processor.connect(sink);
    disconnectGraph = () => {
      try {
        processor.disconnect();
      } catch {}
      try {
        source.disconnect();
      } catch {}
    };
    detachHandler = () => {
      processor.onaudioprocess = null;
    };
  }

  const noAudioTimeout = window.setTimeout(() => {
    if (!gotAnySamples) {
      opts.onError?.(
        new Error(
          "Microphone started but no audio samples were captured. If you're on Safari, try enabling AudioWorklet support or switch to Chrome.",
        ),
      );
    }
  }, 1500);

  const stop = async () => {
    window.clearTimeout(noAudioTimeout);
    detachHandler?.();
    try {
      disconnectGraph?.();
    } catch {}
    try {
      sink.disconnect();
    } catch {}
    try {
      stream.getTracks().forEach((t) => t.stop());
    } catch {}
    try {
      await audioContext.close();
    } catch {}
  };

  return { stop };
}
