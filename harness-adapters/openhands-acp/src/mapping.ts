import { ContentBlock } from "@agentclientprotocol/sdk";

export function contentBlocksToText(blocks: ContentBlock[]): string {
  const parts: string[] = [];

  for (const block of blocks) {
    switch (block.type) {
      case "text":
        parts.push(block.text);
        break;
      case "resource_link": {
        const label = block.title ?? block.name ?? block.uri;
        parts.push(`${label} (${block.uri})`);
        break;
      }
      case "resource": {
        const uri = block.resource.uri;
        if ("text" in block.resource) {
          const header = uri ? `Resource: ${uri}` : "Resource:";
          parts.push(`${header}\n\n${block.resource.text}`);
        } else {
          const header = uri ? `Resource: ${uri}` : "Resource:";
          parts.push(`${header}\n\n[embedded binary content omitted]`);
        }
        break;
      }
      case "image": {
        const label = block.uri ? `Image: ${block.uri}` : "Image";
        parts.push(`${label} (image content omitted)`);
        break;
      }
      case "audio":
        parts.push("Audio content omitted");
        break;
      default: {
        const neverBlock: never = block;
        throw new Error(`Unsupported content block type: ${JSON.stringify(neverBlock)}`);
      }
    }
  }

  return parts.join("\n\n");
}
