execute("INSERT INTO members (age, email) VALUES (18, 'One@example.test')")
check_rejected = false
begin
  execute("INSERT INTO members (age, email) VALUES (17, 'young@example.test')")
rescue StandardError
  check_rejected = true
end
raise "CHECK allowed an underage member" unless check_rejected
duplicate_rejected = false
begin
  execute("INSERT INTO members (age, email) VALUES (20, 'one@example.test')")
rescue StandardError
  duplicate_rejected = true
end
raise "expression index allowed a duplicate" unless duplicate_rejected
execute("INSERT INTO members (age, email) VALUES (NULL, NULL)")
execute("INSERT INTO members (age, email) VALUES (NULL, NULL)")
puts "CHECK and lower(email) enforce writes; NULL is accepted"
