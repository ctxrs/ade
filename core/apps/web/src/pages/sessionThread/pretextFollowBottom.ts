export type ResolveFollowBottomAfterScrollParams = {
  followBottom: boolean;
  previousScrollTop: number;
  currentScrollTop: number;
  bottomOffsetPx: number;
  thresholdPx: number;
  programmaticScroll: boolean;
};

export function resolveFollowBottomAfterScroll({
  followBottom,
  previousScrollTop,
  currentScrollTop,
  bottomOffsetPx,
  thresholdPx,
  programmaticScroll,
}: ResolveFollowBottomAfterScrollParams): boolean {
  const scrolledUp = !programmaticScroll && currentScrollTop < previousScrollTop - 1;
  if (scrolledUp) {
    return false;
  }
  if (bottomOffsetPx <= thresholdPx && (programmaticScroll || bottomOffsetPx <= 1)) {
    return true;
  }
  return followBottom;
}

export function shouldRestoreBottomOnViewportResize(sizeChanged: boolean, followBottom: boolean): boolean {
  return sizeChanged && followBottom;
}
