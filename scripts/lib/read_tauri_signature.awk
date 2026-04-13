BEGIN {
  expect_public_signature = 0
}

function is_signature_payload(value) {
  return value ~ /^[A-Za-z0-9+\/=]+$/ && length(value) >= 64
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
    if (is_signature_payload(line)) {
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
    if (is_signature_payload(line)) {
      print line
      exit
    }
    next
  }

  if (is_signature_payload(line)) {
    print line
    exit
  }
}
