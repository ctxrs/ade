import "react-native";

declare module "react-native" {
  interface ViewStyle {
    [key: string]: any;
  }
  interface TextStyle {
    [key: string]: any;
  }
  interface ImageStyle {
    [key: string]: any;
  }
}
