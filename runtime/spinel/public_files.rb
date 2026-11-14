# frozen_string_literal: true
# The files under public/ that Rails' ActionDispatch::Static serves ahead
# of the routes, by request path. FileHandler tries the path itself, then
# with `.html`, then its `index.html`, so `/about` is `public/about.html`
# and `/docs/` is `public/docs/index.html`.
#
# The file set is listed once, at the first request, rather than stat'ed
# per request: the stat was ~5% of a `/up` request on the bench box, paid
# by every route. A page the app writes later through the page cache
# (runtime/action_controller_page_caching.rb) is added as it is written
# and removed as it is expired. A path the set names is confirmed on disk
# before it is served, so a file another worker expired, or one expired
# while this one was being written, falls through to the routes instead of
# answering 404. When the page cache writes under public/, a miss also
# looks at the disk, which is how a page another worker cached is found.
#
# The page cache writes only through `cache_page` (`caches_page` expands to
# it). When nothing the app emits names it, boot.rb says so
# (`no_page_writes!`, src/project.rs), and a miss skips the two names only
# the page cache gives a path (`.html`, `/index.html`): two `File.file?` per
# GET that is not a public file. The path itself is still looked up on
# disk, as Rails' FileHandler does: something other than the page cache can
# write there while the app runs (an upload stored under public/, as
# Paperclip's `public/system` and Discourse's `public/uploads`; a sitemap
# job), and that file is served at once.
module ActionController
  module PublicFiles
    @page_writes = true

    def self.no_page_writes!
      @page_writes = false
      @watch_disk = nil
      nil
    end

    def self.files
      if @files.nil?
        @files = {}
        Dir.glob("public/**/*", File::FNM_DOTMATCH).each do |path|
          ActionController::PublicFiles.add(path["public/".length, path.length]) if File.file?(path)
        end
      end
      @files
    end

    # The file a GET/HEAD for `path` is answered with, as a decoded path
    # under public/ (FileHandler unescapes the request path first, and the
    # page cache writes the decoded name), or "" for none.
    def self.lookup(path)
      decoded = ActionDispatch::Router.decode_capture(path)
      return "" if decoded.include?("..") || decoded.include?("\0")
      stem = decoded.chomp("/")
      found = ActionController::PublicFiles.present(decoded, ActionController::PublicFiles.watch_disk?)
      found = ActionController::PublicFiles.present(stem + ".html", ActionController::PublicFiles.watch_pages?) if found.empty? && !stem.empty?
      found = ActionController::PublicFiles.present(stem + "/index.html", ActionController::PublicFiles.watch_pages?) if found.empty?
      found
    end

    # `file` when it is on disk and the set names it, or, for a name the set
    # lacks, when `on_disk` and it is on disk (another worker's page, a file
    # written there while the app runs); else "".
    def self.present(file, on_disk)
      files = ActionController::PublicFiles.files
      if files.key?(file)
        return file if File.file?("public" + file)
        files.delete(file)
        return ""
      end
      return "" unless on_disk
      return "" unless File.file?("public" + file)
      files[file] = true
      file
    end

    # Whether a file can appear under public/ while the app runs: the page
    # cache is on and writes there (production's default). Only then does a
    # miss look at the disk, so an app that does not cache into public/
    # stats nothing per route.
    def self.watch_disk?
      if @watch_disk.nil?
        public_dir = File.expand_path("public")
        cache_dir = File.expand_path(ActionController::Base.page_cache_directory)
        @watch_disk = ActionController::Base.perform_caching &&
          (cache_dir == public_dir || cache_dir.start_with?(public_dir + "/"))
      end
      @watch_disk
    end

    # Whether a miss also looks on disk for the page cache's own names (the
    # path with `.html`, its `/index.html`): only when the app writes pages.
    def self.watch_pages?
      @page_writes && ActionController::PublicFiles.watch_disk?
    end

    # Forget what the first lookup decided, after the settings change (the tests).
    def self.reset_watch!
      @watch_disk = nil
      nil
    end

    def self.add(rel)
      ActionController::PublicFiles.files["/" + rel] = true
      nil
    end

    def self.remove(rel)
      ActionController::PublicFiles.files.delete("/" + rel)
      nil
    end

    # `disk` relative to public/, or "" when it lies outside it.
    def self.relative(disk)
      root = File.join(File.expand_path("public"), "")
      return "" unless disk.start_with?(root)
      disk[root.length, disk.length - root.length]
    end
  end
end
