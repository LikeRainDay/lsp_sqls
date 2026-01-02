SELECT * FROM users PARTITION (dt='2024-01-01');
SELECT id, name FROM users WHERE dt = '2024-01-01';
