{
  line = $0
  gsub(/\r/, "", line)

  if (line ~ /^Signature:[[:space:]]*/) {
    sub(/^Signature:[[:space:]]*/, "", line)
    if (line != "") {
      print line
      exit
    }
  }

  if (line ~ /^RW[A-Za-z0-9+\/=:-]{64,}$/) {
    print line
    exit
  }
}
