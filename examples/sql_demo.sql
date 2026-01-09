-- Sample SQL queries to test LSP completions
-- Use these examples in the interactive client to verify functionality

-- Example 1: Basic SELECT with WHERE clause
SELECT * FROM users WHERE

-- Example 2: Table-specific column completion
SELECT u. FROM users u

-- Example 3: JOIN with multiple tables
SELECT u.name, o.total
FROM users u
JOIN orders o ON u.id = o.user_id
WHERE

-- Example 4: Aggregate functions with GROUP BY
SELECT user_id, COUNT(*), SUM(total)
FROM orders
GROUP BY

-- Example 5: HAVING clause
SELECT user_id, COUNT(*) as order_count
FROM orders
GROUP BY user_id
HAVING

-- Example 6: ORDER BY
SELECT * FROM users
ORDER BY

-- Example 7: Subquery
SELECT * FROM users
WHERE id IN (
  SELECT user_id FROM orders WHERE
)

-- Example 8: Multiple JOINs
SELECT
  u.name,
  o.id as order_id,
  o.total
FROM users u
LEFT JOIN orders o ON u.id = o.
WHERE u.

-- Example 9: Complex WHERE with operators
SELECT * FROM orders
WHERE status

-- Example 10: Column aliases
SELECT
  user_id as uid,
  COUNT(*) as total_orders,
  SUM(total) as revenue
FROM orders
GROUP BY user_id
HAVING
