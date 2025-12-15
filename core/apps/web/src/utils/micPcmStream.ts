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

  const stream = await navigator.mediaDevices.getUserMedia({ audio: true });

  const audioContext = new AudioContext({ sampleRate: DEFAULT_TARGET_SAMPLE_RATE });
  const source = audioContext.createMediaStreamSource(stream);

  const workletCode = `
    class MicProcessor extends AudioWorkletProcessor {
      process(inputs) {
        const input = inputs[0];
        if (input && input[0] && input[0].length) {
          this.port.postMessage(input[0]);
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
  // Not connecting to destination avoids audible feedback.

  let pcmCarry = new Int16Array(0);

  const onMessage = (ev: MessageEvent) => {
    try {
      const input = ev.data as Float32Array;
      if (!(input instanceof Float32Array)) return;

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
    } catch (e: any) {
      opts.onError?.(e instanceof Error ? e : new Error(String(e)));
    }
  };

  node.port.addEventListener("message", onMessage);
  node.port.start();

  const stop = async () => {
    node.port.removeEventListener("message", onMessage);
    try {
      node.disconnect();
    } catch {}
    try {
      source.disconnect();
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

