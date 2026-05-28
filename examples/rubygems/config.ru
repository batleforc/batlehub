require "bundler/setup"
require "rack"

app = Rack::Builder.new do
  map "/" do
    run ->(env) { [200, { "Content-Type" => "application/json" }, ['{"message":"hello"}']] }
  end
end

run app
