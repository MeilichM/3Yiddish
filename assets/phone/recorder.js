// Runs on the audio thread: hands the microphone's samples to the page, a block at a time.
class Take extends AudioWorkletProcessor {
  constructor() {
    super();
    this.block = new Float32Array(2048);
    this.filled = 0;
  }
  process(inputs) {
    const input = inputs[0] && inputs[0][0];
    if (!input) return true;
    let at = 0;
    while (at < input.length) {
      const room = Math.min(input.length - at, this.block.length - this.filled);
      this.block.set(input.subarray(at, at + room), this.filled);
      this.filled += room;
      at += room;
      if (this.filled === this.block.length) {
        this.port.postMessage(this.block.slice(0));
        this.filled = 0;
      }
    }
    return true;
  }
}
registerProcessor('take', Take);
