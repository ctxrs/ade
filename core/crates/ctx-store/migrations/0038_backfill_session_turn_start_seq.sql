WITH sessions_missing_seq AS (
  SELECT session_id
  FROM session_turns
  GROUP BY session_id
  HAVING SUM(CASE WHEN start_seq IS NOT NULL THEN 1 ELSE 0 END) = 0
),
ordered AS (
  SELECT
    st.turn_id,
    st.session_id,
    ROW_NUMBER() OVER (
      PARTITION BY st.session_id
      ORDER BY st.started_at ASC, st.turn_id ASC
    ) AS rn
  FROM session_turns st
  INNER JOIN sessions_missing_seq sms
    ON sms.session_id = st.session_id
)
UPDATE session_turns
SET start_seq = (
  SELECT rn
  FROM ordered
  WHERE ordered.turn_id = session_turns.turn_id
    AND ordered.session_id = session_turns.session_id
)
WHERE session_id IN (SELECT session_id FROM sessions_missing_seq)
  AND start_seq IS NULL;
