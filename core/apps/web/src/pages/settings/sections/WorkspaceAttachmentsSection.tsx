import type { ReactNode } from "react";

type SectionProps = {
  children: ReactNode;
};

export function WorkspaceAttachmentsSection({ children }: SectionProps) {
  return <>{children}</>;
}
