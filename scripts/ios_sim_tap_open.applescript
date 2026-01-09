on run argv
  set action to "open"
  if (count of argv) >= 1 then
    set action to item 1 of argv
  end if

  if action is "open" then
    my clickOpenPrompt()
    return
  end if

  tell application "Simulator" to activate
  delay 0.2

  if action is "shake" then
    my triggerShake()
    return
  end if

  set frame to my findScreenFrame()
  if frame is {} then
    return
  end if

  set baseX to item 1 of frame
  set baseY to item 2 of frame
  set width to item 3 of frame
  set height to item 4 of frame

  if action is "tap" then
    if (count of argv) < 3 then
      return
    end if
    set xVal to item 2 of argv as real
    set yVal to item 3 of argv as real
    set clickX to baseX + (xVal * width)
    set clickY to baseY + (yVal * height)
    my clickAt(clickX, clickY)
    return
  end if

  if action is "swipe" then
    if (count of argv) < 5 then
      return
    end if
    set x1 to item 2 of argv as real
    set y1 to item 3 of argv as real
    set x2 to item 4 of argv as real
    set y2 to item 5 of argv as real
    set startX to baseX + (x1 * width)
    set startY to baseY + (y1 * height)
    set endX to baseX + (x2 * width)
    set endY to baseY + (y2 * height)
    my dragFromTo(startX, startY, endX, endY)
    return
  end if
end run

on clickOpenPrompt()
  tell application "Simulator" to activate
  set timeoutSeconds to 20
  repeat timeoutSeconds times
    tell application "System Events"
      if exists (button "Open" of sheet 1 of window 1 of process "Simulator") then
        click button "Open" of sheet 1 of window 1 of process "Simulator"
        exit repeat
      end if
    end tell
    delay 1
  end repeat
end clickOpenPrompt

on triggerShake()
  try
    tell application "System Events"
      tell process "Simulator"
        click menu item "Shake" of menu "Device" of menu bar 1
      end tell
    end tell
  end try
end triggerShake

on findScreenFrame()
  tell application "System Events"
    tell process "Simulator"
      if not (exists window 1) then
        return {}
      end if
      set simWindow to window 1
      set bestArea to 0
      set bestFrame to {}
      set elements to UI elements of simWindow
      repeat with el in elements
        try
          set elPos to position of el
          set elSize to size of el
          set elArea to (item 1 of elSize) * (item 2 of elSize)
          if elArea > bestArea then
            set bestArea to elArea
            set bestFrame to {item 1 of elPos, item 2 of elPos, item 1 of elSize, item 2 of elSize}
          end if
        end try
      end repeat
      if bestArea is 0 then
        set winPos to position of simWindow
        set winSize to size of simWindow
        set bestFrame to {item 1 of winPos, item 2 of winPos, item 1 of winSize, item 2 of winSize}
      end if
      return bestFrame
    end tell
  end tell
end findScreenFrame

on clickAt(xPos, yPos)
  tell application "System Events"
    click at {xPos, yPos}
  end tell
end clickAt

on dragFromTo(startX, startY, endX, endY)
  try
    tell application "System Events"
      drag from {startX, startY} to {endX, endY}
    end tell
  end try
end dragFromTo
