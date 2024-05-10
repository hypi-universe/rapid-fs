export default async function () {
  let stream = file.open('file1.txt', {read: true})
  let buf = new Uint8Array(0)
  let idx = 0;
  for await (const chunk of stream) {
    let buffer = buf.buffer.transfer(buf.byteLength + chunk.byteLength);
    buf = new Uint8Array(buffer);
    buf.set(chunk, idx);
    idx = idx + chunk.length;
  }
  let decoder = new TextDecoder();
  return decoder.decode(buf);
}
