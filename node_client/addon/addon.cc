#include <napi.h>
#include <string>

extern "C" {
  char* prosperous_initialize(const char* key, const char* base_url);
  void  prosperous_free_string(char* s);
}

// Runs prosperous_initialize on a libuv thread pool thread and resolves a
// Promise on the JS event loop. This ensures the blocking Rust call (which
// drives a tokio runtime internally) never stalls the Node.js event loop.
class InitializeWorker : public Napi::AsyncWorker {
 public:
  InitializeWorker(Napi::Env env,
                   Napi::Promise::Deferred deferred,
                   std::string key,
                   std::string base_url)
      : Napi::AsyncWorker(env),
        deferred_(deferred),
        key_(std::move(key)),
        base_url_(std::move(base_url)),
        result_(nullptr) {}

  void Execute() override {
    const char* key_ptr      = key_.empty()      ? nullptr : key_.c_str();
    const char* base_url_ptr = base_url_.empty() ? nullptr : base_url_.c_str();
    result_ = prosperous_initialize(key_ptr, base_url_ptr);
  }

  void OnOK() override {
    Napi::HandleScope scope(Env());
    Napi::String json = Napi::String::New(Env(), result_ ? result_ : "{}");
    prosperous_free_string(result_);
    result_ = nullptr;
    deferred_.Resolve(json);
  }

  void OnError(const Napi::Error& e) override {
    deferred_.Reject(e.Value());
  }

 private:
  Napi::Promise::Deferred deferred_;
  std::string key_;
  std::string base_url_;
  char* result_;
};

// initialize(options) -> Promise<string>
// options: { prosperousKey?: string, baseUrl?: string }
Napi::Value Initialize(const Napi::CallbackInfo& info) {
  Napi::Env env = info.Env();
  auto deferred = Napi::Promise::Deferred::New(env);

  std::string key;
  std::string base_url;

  if (info.Length() > 0 && info[0].IsObject()) {
    Napi::Object opts = info[0].As<Napi::Object>();

    if (opts.Has("prosperousKey") && opts.Get("prosperousKey").IsString()) {
      key = opts.Get("prosperousKey").As<Napi::String>().Utf8Value();
    }
    if (opts.Has("baseUrl") && opts.Get("baseUrl").IsString()) {
      base_url = opts.Get("baseUrl").As<Napi::String>().Utf8Value();
    }
  }

  auto* worker = new InitializeWorker(env, deferred, key, base_url);
  worker->Queue();
  return deferred.Promise();
}

Napi::Object Init(Napi::Env env, Napi::Object exports) {
  exports.Set("initialize", Napi::Function::New(env, Initialize));
  return exports;
}

NODE_API_MODULE(prosperous, Init)
