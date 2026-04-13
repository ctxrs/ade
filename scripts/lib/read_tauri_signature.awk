BEGIN {
  expect_public_signature = 0
}

{
  line = $0
  gsub(/\033\[[0-9;]*[[:alpha:]]/, "", line)
  gsub(/\r/, "", line)
  sub(/^[[:space:]]+/, "", line)
  sub(/[[:space:]]+$/, "", line)

  if (line == "") {
    next
  }

  if (expect_public_signature == 1) {
    if (line ~ /^[A-Za-z0-9+\/=]{64,}$/) {
      print line
      exit
    }
    expect_public_signature = 0
  }

  if (line ~ /^Public signature:[[:space:]]*$/) {
    expect_public_signature = 1
    next
  }

  if (line ~ /^Signature:[[:space:]]*/) {
    sub(/^Signature:[[:space:]]*/, "", line)
    if (line ~ /^[A-Za-z0-9+\/=]{64,}$/) {
      print line
      exit
    }
    next
  }

  if (line ~ /^[A-Za-z0-9+\/=]{64,}$/) {
    print line
    exit
  }
}
