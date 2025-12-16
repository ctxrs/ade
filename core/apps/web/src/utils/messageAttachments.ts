import { uploadBlob, type MessageAttachment } from "../api/client";

const IMAGE_EXT_RE = /\.(avif|bmp|gif|jpe?g|png|svg|tiff?|webp)$/i;

export function isImageFile(file: File): boolean {
  const type = (file.type || "").toLowerCase();
  if (type.startsWith("image/")) return true;
  const name = (file.name || "").trim();
  if (!name) return false;
  return IMAGE_EXT_RE.test(name);
}

async function fileToDataUrl(file: File): Promise<string> {
  return await new Promise<string>((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(new Error("file read failed"));
    reader.onload = () => resolve(String(reader.result ?? ""));
    reader.readAsDataURL(file);
  });
}

export async function imageFilesToInlineAttachments(files: File[]): Promise<MessageAttachment[]> {
  const next: MessageAttachment[] = [];
  for (const file of files) {
    if (!isImageFile(file)) continue;
    const dataUrl = await fileToDataUrl(file);
    const idx = dataUrl.indexOf("base64,");
    if (idx === -1) continue;
    next.push({
      kind: "image",
      mime_type: file.type || "image/*",
      data_base64: dataUrl.slice(idx + "base64,".length),
      name: file.name,
    });
  }
  return next;
}

export async function imageFilesToBlobRefAttachments(files: File[]): Promise<MessageAttachment[]> {
  const next: MessageAttachment[] = [];
  for (const file of files) {
    if (!isImageFile(file)) continue;
    const uploaded = await uploadBlob(file);
    next.push({
      kind: "image_ref",
      blob_id: uploaded.blob_id,
      mime_type: uploaded.mime_type,
      name: uploaded.name ?? file.name,
    });
  }
  return next;
}

