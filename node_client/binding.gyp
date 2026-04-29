{
  "targets": [{
    "target_name": "prosperous",
    "sources": ["addon/addon.cc"],
    "include_dirs": ["<!@(node -p \"require('node-addon-api').include\")"],
    "defines": ["NAPI_DISABLE_CPP_EXCEPTIONS"],
    "conditions": [
      ["OS=='win'", {
        "libraries": [
          "<(module_root_dir)/../target/release/prosperous_client_native.lib"
        ],
        "msvs_settings": {
          "VCLinkerTool": {
            "AdditionalDependencies": ["userenv.lib", "ntdll.lib", "ws2_32.lib"]
          }
        }
      }],
      ["OS=='mac'", {
        "libraries": [
          "<(module_root_dir)/../target/release/libprosperous_client_native.a"
        ],
        "xcode_settings": {
          "OTHER_LDFLAGS": [
            "-framework SystemConfiguration",
            "-framework Security",
            "-framework CoreFoundation"
          ]
        }
      }],
      ["OS=='linux'", {
        "libraries": [
          "<(module_root_dir)/../target/release/libprosperous_client_native.a"
        ],
        "ldflags": ["-lpthread", "-ldl"]
      }]
    ]
  }]
}
