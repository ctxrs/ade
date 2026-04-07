import { uploadBlob, type MessageAttachment } from "../api/client";

const IMAGE_EXT_RE = /\.(avif|bmp|gif|jpe?g|png|svg|tiff?|webp)$/i;
export const MAX_MESSAGE_IMAGE_ATTACHMENT_BYTES = 25 * 1024 * 1024;
const MAX_MESSAGE_IMAGE_ATTACHMENT_MIB = MAX_MESSAGE_IMAGE_ATTACHMENT_BYTES / (1024 * 1024);

export function isImageFile(file: File): boolean {
  const type = (file.type || "").toLowerCase();
  if (type.startsWith("image/")) return true;
  const name = (file.name || "").trim();
  if (!name) return false;
  return IMAGE_EXT_RE.test(name);
}

export function imageAttachmentSizeError(name?: string | null): string {
  const label = (name ?? "").trim();
  const prefix = label ? `${label} is too large.` : "Image attachment is too large.";
  return `${prefix} Image attachments must be ${MAX_MESSAGE_IMAGE_ATTACHMENT_MIB} MiB or smaller.`;
}

function assertSupportedImageFileSize(file: File): void {
  if (file.size > MAX_MESSAGE_IMAGE_ATTACHMENT_BYTES) {
    throw new Error(imageAttachmentSizeError(file.name));
  }
}

export async function imageFilesToBlobRefAttachments(files: File[]): Promise<MessageAttachment[]> {
  const imageFiles = files.filter(isImageFile);
  for (const file of imageFiles) {
    assertSupportedImageFileSize(file);
  }

  const next: MessageAttachment[] = [];
  for (const file of imageFiles) {
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

export async function imageFilesToMessageAttachments(files: File[]): Promise<MessageAttachment[]> {
  return imageFilesToBlobRefAttachments(files);
}
