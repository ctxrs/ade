declare module "@context/design/native" {
  import { StyleSheet } from "react-native";
  import type { ContextTokens } from "@context/design";

  export { tokens } from "@context/design";
  export type { ContextTokens };

  export function useContextTokens(): ContextTokens;
  export function createContextStyles<T extends Record<string, any>>(
    factory: (theme: ContextTokens) => T,
  ): T;
}
