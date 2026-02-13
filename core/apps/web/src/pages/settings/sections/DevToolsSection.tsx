import type { ReactNode } from "react";

type SectionProps = {
  children: ReactNode;
};

export function DevToolsSection({ children }: SectionProps) {
  return <>{children}</>;
}
