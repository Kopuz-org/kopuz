// C++ host for the system Widevine CDM, driven through its official interface
// (cdm::ContentDecryptionModule_11 + cdm::Host_11).

#include "content_decryption_module.h"

#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <chrono>
#include <string>
#include <vector>
#include <utility>

#if defined(_WIN32)
#include <windows.h>
#else
#include <dlfcn.h>
#endif

using namespace cdm;

namespace {

// The CDM ships as a plain shared library on every desktop platform; only the
// loader API differs.

#if defined(_WIN32)
using LibHandle = HMODULE;

LibHandle lib_open(const char* path) {
  // The CDM path can contain non-ASCII (a Windows user profile name), so widen
  // it rather than relying on the ANSI code page.
  int n = MultiByteToWideChar(CP_UTF8, 0, path, -1, nullptr, 0);
  if (n <= 0) return nullptr;
  std::vector<wchar_t> wide(static_cast<size_t>(n));
  if (MultiByteToWideChar(CP_UTF8, 0, path, -1, wide.data(), n) <= 0) return nullptr;
  return LoadLibraryW(wide.data());
}

void* lib_sym(LibHandle h, const char* name) {
  return reinterpret_cast<void*>(GetProcAddress(h, name));
}
#else
using LibHandle = void*;

LibHandle lib_open(const char* path) { return dlopen(path, RTLD_NOW | RTLD_GLOBAL); }
void* lib_sym(LibHandle h, const char* name) { return dlsym(h, name); }
#endif

// Heap-backed cdm::Buffer for the CDM's output.
class HeapBuffer : public Buffer {
 public:
  explicit HeapBuffer(uint32_t cap) : data_(cap), size_(0) {}
  void Destroy() override { delete this; }
  uint32_t Capacity() const override { return static_cast<uint32_t>(data_.size()); }
  uint8_t* Data() override { return data_.data(); }
  void SetSize(uint32_t size) override { size_ = size; }
  uint32_t Size() const override { return size_; }

 private:
  std::vector<uint8_t> data_;
  uint32_t size_;
};

class Host : public Host_11 {
 public:
  ContentDecryptionModule_11* cdm = nullptr;

  // The CDM answers asynchronously through these callbacks; we drive it from a
  // single thread and spin on the flags below rather than run an event loop.
  bool initialized = false, init_ok = false;
  std::string session_id;
  bool got_message = false;
  uint32_t message_type = 0;
  std::vector<uint8_t> challenge;
  bool rejected = false;
  std::string error;
  bool keys_changed = false;
  bool session_closed = false;
  bool resolved = false;
  std::vector<std::pair<int64_t, void*>> timers;

  void fire_timers() {
    auto t = timers;
    timers.clear();
    for (auto& e : t)
      if (cdm) cdm->TimerExpired(e.second);
  }

  // --- cdm::Host_11 ---
  Buffer* Allocate(uint32_t capacity) override { return new HeapBuffer(capacity); }
  void SetTimer(int64_t delay_ms, void* context) override { timers.push_back({delay_ms, context}); }
  Time GetCurrentWallTime() override {
    using namespace std::chrono;
    return duration<double>(system_clock::now().time_since_epoch()).count();
  }
  void OnInitialized(bool success) override { initialized = true; init_ok = success; }
  void OnResolveKeyStatusPromise(uint32_t, KeyStatus) override { resolved = true; }
  void OnResolveNewSessionPromise(uint32_t, const char* sid, uint32_t n) override {
    session_id.assign(sid, n);
    resolved = true;
  }
  void OnResolvePromise(uint32_t) override { resolved = true; }
  void OnRejectPromise(uint32_t, Exception, uint32_t, const char* msg, uint32_t n) override {
    rejected = true;
    if (msg && n) error.assign(msg, n);
  }
  void OnSessionMessage(const char*, uint32_t, MessageType type, const char* msg,
                        uint32_t n) override {
    got_message = true;
    // Kept, not discarded: an unprovisioned CDM answers with an
    // individualization request for Google's provisioning server, which is not
    // a licence challenge and must not be sent to the content licence server.
    message_type = static_cast<uint32_t>(type);
    challenge.assign(reinterpret_cast<const uint8_t*>(msg),
                     reinterpret_cast<const uint8_t*>(msg) + n);
  }
  void OnSessionKeysChange(const char*, uint32_t, bool, const KeyInformation*, uint32_t) override {
    keys_changed = true;
  }
  void OnExpirationChange(const char*, uint32_t, Time) override {}
  void OnSessionClosed(const char*, uint32_t) override { session_closed = true; }
  void SendPlatformChallenge(const char*, uint32_t, const char*, uint32_t) override {}
  void EnableOutputProtection(uint32_t) override {}
  void QueryOutputProtectionStatus() override {}
  void OnDeferredInitializationDone(StreamType, Status) override {}
  FileIO* CreateFileIO(FileIOClient*) override { return nullptr; }
  void RequestStorageId(uint32_t version) override { (void)version; }
  void ReportMetrics(MetricName, uint64_t) override {}
};

Host* g_host = nullptr;

void* GetHost(int version, void* /*user_data*/) {
  if (version == Host_11::kVersion) return static_cast<Host_11*>(g_host);
  return nullptr;
}

// Copy `src` into a malloc'd buffer the Rust side frees via wv_free.
bool emit(const uint8_t* src, size_t n, uint8_t** out, uint32_t* out_len) {
  *out_len = static_cast<uint32_t>(n);
  *out = static_cast<uint8_t*>(malloc(n ? n : 1));
  if (!*out) return false;
  if (n) memcpy(*out, src, n);
  return true;
}

}  // namespace

extern "C" {

// 0 = success; non-zero maps to WidevineError in the Rust wrapper.
int wv_open(const char* so_path) {
  if (g_host && g_host->cdm) return 0;

  LibHandle lib = lib_open(so_path);
  if (!lib) return 1;
  auto init = reinterpret_cast<void (*)()>(lib_sym(lib, "InitializeCdmModule_4"));
  auto create = reinterpret_cast<void* (*)(int, const char*, uint32_t, GetCdmHostFunc, void*)>(
      lib_sym(lib, "CreateCdmInstance"));
  if (!init || !create) return 2;
  init();
  delete g_host;
  g_host = new Host();
  const char* ks = "com.widevine.alpha";
  void* inst = create(11, ks, static_cast<uint32_t>(strlen(ks)), GetHost, nullptr);
  if (!inst) return 3;
  g_host->cdm = static_cast<ContentDecryptionModule_11*>(inst);
  // No distinctive identifier and no persistent state: we only need a temporary
  // session, and both would otherwise want storage we don't provide (CreateFileIO
  // returns null).
  g_host->cdm->Initialize(/*allow_distinctive_identifier=*/false,
                          /*allow_persistent_state=*/false,
                          /*use_hw_secure_codecs=*/false);
  for (int i = 0; i < 200 && !g_host->initialized; ++i) g_host->fire_timers();
  return g_host->initialized && g_host->init_ok ? 0 : 4;
}

// init_data is a CENC pssh box. Returns the license challenge in *out.
// Opens a CDM session and returns its licence challenge plus the session id.
//
// The id is the caller's to keep: the CDM holds one set of content keys per
// session and picks between them by key id, so a track can only be decrypted
// while its session is open. Closing them is wv_close's job, once the track is
// done — not here.
int wv_challenge(const uint8_t* init_data, uint32_t len, uint8_t** out, uint32_t* out_len,
                 uint32_t* out_type, uint8_t** out_sid, uint32_t* out_sid_len) {
  if (!g_host || !g_host->cdm) return 10;
  g_host->got_message = false;
  g_host->rejected = false;
  g_host->message_type = 0;
  g_host->challenge.clear();
  g_host->session_id.clear();
  g_host->cdm->CreateSessionAndGenerateRequest(1, SessionType::kTemporary, InitDataType::kCenc,
                                               init_data, len);
  for (int i = 0; i < 500 && !g_host->got_message && !g_host->rejected; ++i) g_host->fire_timers();
  if (g_host->rejected) return 11;
  if (!g_host->got_message) return 12;
  if (g_host->session_id.empty()) return 14;
  if (out_type) *out_type = g_host->message_type;
  if (!emit(reinterpret_cast<const uint8_t*>(g_host->session_id.data()),
            g_host->session_id.size(), out_sid, out_sid_len)) {
    return 13;
  }
  if (!emit(g_host->challenge.data(), g_host->challenge.size(), out, out_len)) {
    free(*out_sid);
    *out_sid = nullptr;
    return 13;
  }
  return 0;
}

// Release a session and the keys it holds. Anything still decrypting against
// those keys must be finished first.
int wv_close(const uint8_t* sid, uint32_t sid_len) {
  if (!g_host || !g_host->cdm) return 40;
  g_host->session_closed = false;
  g_host->rejected = false;
  g_host->cdm->CloseSession(3, reinterpret_cast<const char*>(sid), sid_len);
  for (int i = 0; i < 500 && !g_host->session_closed && !g_host->rejected; ++i) {
    g_host->fire_timers();
  }
  if (g_host->rejected) return 41;
  return g_host->session_closed ? 0 : 42;
}

// Feed the license response back so the CDM learns the content keys.
int wv_update(const uint8_t* sid, uint32_t sid_len, const uint8_t* license, uint32_t len) {
  if (!g_host || !g_host->cdm) return 20;
  g_host->keys_changed = false;
  g_host->rejected = false;
  g_host->cdm->UpdateSession(2, reinterpret_cast<const char*>(sid), sid_len, license, len);
  for (int i = 0; i < 500 && !g_host->keys_changed && !g_host->rejected; ++i) g_host->fire_timers();
  if (g_host->rejected) return 21;
  return g_host->keys_changed ? 0 : 22;
}

// Decrypt one CENC buffer. subs is a flattened [clear0,cipher0,clear1,...] array
// of num_subs pairs; num_subs == 0 means the whole buffer is encrypted.
int wv_decrypt(const uint8_t* data, uint32_t data_size, const uint8_t* key_id,
               uint32_t key_id_size, const uint8_t* iv, uint32_t iv_size, const uint32_t* subs,
               uint32_t num_subs, uint8_t** out, uint32_t* out_len) {
  if (!g_host || !g_host->cdm) return 30;
  std::vector<SubsampleEntry> subsamples;
  subsamples.reserve(num_subs);
  for (uint32_t i = 0; i < num_subs; ++i) {
    SubsampleEntry e;
    e.clear_bytes = subs[i * 2];
    e.cipher_bytes = subs[i * 2 + 1];
    subsamples.push_back(e);
  }
  InputBuffer_2 in;
  memset(&in, 0, sizeof(in));
  in.data = data;
  in.data_size = data_size;
  in.encryption_scheme = EncryptionScheme::kCenc;
  in.key_id = key_id;
  in.key_id_size = key_id_size;
  in.iv = iv;
  in.iv_size = iv_size;
  in.subsamples = subsamples.empty() ? nullptr : subsamples.data();
  in.num_subsamples = num_subs;

  class Block : public DecryptedBlock {
   public:
    Buffer* buf = nullptr;
    int64_t ts = 0;
    void SetDecryptedBuffer(Buffer* b) override { buf = b; }
    Buffer* DecryptedBuffer() override { return buf; }
    void SetTimestamp(int64_t t) override { ts = t; }
    int64_t Timestamp() const override { return ts; }
  } block;

  Status s = g_host->cdm->Decrypt(in, &block);
  if (s != Status::kSuccess) return 31 + static_cast<int>(s);
  Buffer* b = block.DecryptedBuffer();
  if (!b) return 40;
  bool ok = emit(b->Data(), b->Size(), out, out_len);
  b->Destroy();
  return ok ? 0 : 41;
}

void wv_free(uint8_t* p) { free(p); }

}  // extern "C"
