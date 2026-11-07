ActiveRecord::Schema[8.1].define(version: 1) do
  create_table "members" do |t|
    t.integer "age"
    t.string "email"
    t.check_constraint "age >= 18", name: "members_adult"
    t.index "lower(email)", unique: true, name: "members_email_lower"
  end
end
