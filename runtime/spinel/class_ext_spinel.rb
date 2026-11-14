# frozen_string_literal: true

# ActiveSupport's class-side introspection for the Spinel tree: `Class#descendants`
# (activesupport 8.1 `core_ext/class/subclasses.rb`) and `module_parent_name` /
# `module_parent` / `module_parents` (`core_ext/module/introspection.rb`), MIT
# license, Copyright (c) David Heinemeier Hansson. Spinel answers a reopened
# `Module`'s methods on no class, so these are `Class`'s: a module's own
# `module_parents` is not answered. Rails memoizes the parent name; this reads it
# each time, which answers the same.
class Class
  def descendants
    found = subclasses
    found.each { |klass| found = found + klass.descendants }
    found
  end

  def module_parent_name
    full = name
    return nil if full.nil?
    at = full.rindex("::")
    at.nil? ? nil : full[0, at]
  end

  def module_parent
    parent = module_parent_name
    parent.nil? ? Object : Object.const_get(parent)
  end

  def module_parents
    parents = []
    parts = module_parent_name.to_s.split("::")
    until parts.empty?
      parents << Object.const_get(parts.join("::"))
      parts.pop
    end
    parents << Object unless parents.include?(Object)
    parents
  end
end
