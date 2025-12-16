import {
  Archive,
  ArrowUp,
  AtSign,
  ChevronDown,
  Ellipsis,
  Image,
  Info,
  Laptop,
  MessageSquare,
  Mic,
  Settings,
  Slash,
  Square,
} from "lucide-react";

export function IconChevronDown({ size = 16 }: { size?: number }) {
  return <ChevronDown size={size} />;
}

export function IconArrowUp({ size = 16 }: { size?: number }) {
  return <ArrowUp size={size} />;
}

export function IconStop({ size = 16 }: { size?: number }) {
  return <Square size={size} />;
}

export function IconAt({ size = 16 }: { size?: number }) {
  return <AtSign size={size} />;
}

export function IconSlash({ size = 16 }: { size?: number }) {
  return <Slash size={size} />;
}

export function IconImage({ size = 16 }: { size?: number }) {
  return <Image size={size} />;
}

export function IconMic({ size = 16 }: { size?: number }) {
  return <Mic size={size} />;
}

export function IconLaptop({ size = 16 }: { size?: number }) {
  return <Laptop size={size} />;
}

export function IconInfo({ size = 16 }: { size?: number }) {
  return <Info size={size} />;
}

export function IconGear({ size = 16 }: { size?: number }) {
  return <Settings size={size} />;
}

export function IconDots({ size = 16 }: { size?: number }) {
  return <Ellipsis size={size} />;
}

export function IconArchive({ size = 16 }: { size?: number }) {
  return <Archive size={size} />;
}

export function IconChat({ size = 16 }: { size?: number }) {
  return <MessageSquare size={size} />;
}
