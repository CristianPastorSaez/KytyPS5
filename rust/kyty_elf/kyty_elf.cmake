# Builds the Rust kyty_elf crate with cargo and exposes it as the imported static library
# `kyty_elf`, which carries its include dir and the KYTY_RUST_ELF compile definition.
# Falls back to the C++ header reader when cargo is not available.

option(KYTY_RUST_ELF "Read ELF/SELF header tables with the Rust kyty_elf crate" ON)

if(KYTY_RUST_ELF)
	find_program(KYTY_CARGO cargo HINTS "$ENV{CARGO_HOME}/bin" "$ENV{USERPROFILE}/.cargo/bin" "$ENV{HOME}/.cargo/bin")
	if(NOT KYTY_CARGO)
		message(WARNING "cargo not found: KYTY_RUST_ELF disabled, using the C++ ELF header reader")
		set(KYTY_RUST_ELF OFF)
	endif()
endif()

if(KYTY_RUST_ELF)
	set(kyty_elf_dir "${CMAKE_CURRENT_LIST_DIR}")
	set(kyty_elf_target_dir "${CMAKE_BINARY_DIR}/rust")
	if(WIN32)
		set(kyty_elf_lib "${kyty_elf_target_dir}/release/kyty_elf.lib")
	else()
		set(kyty_elf_lib "${kyty_elf_target_dir}/release/libkyty_elf.a")
	endif()
	file(GLOB kyty_elf_sources CONFIGURE_DEPENDS "${kyty_elf_dir}/src/*.rs")

	add_custom_command(
		OUTPUT "${kyty_elf_lib}"
		COMMAND ${CMAKE_COMMAND} -E env "CARGO_TARGET_DIR=${kyty_elf_target_dir}"
		        "${KYTY_CARGO}" build --release --locked --manifest-path "${kyty_elf_dir}/Cargo.toml"
		DEPENDS ${kyty_elf_sources} "${kyty_elf_dir}/Cargo.toml" "${kyty_elf_dir}/Cargo.lock"
		COMMENT "cargo build kyty_elf"
		VERBATIM)
	add_custom_target(kyty_elf_build DEPENDS "${kyty_elf_lib}")

	add_library(kyty_elf STATIC IMPORTED GLOBAL)
	set_target_properties(kyty_elf PROPERTIES
		IMPORTED_LOCATION "${kyty_elf_lib}"
		INTERFACE_INCLUDE_DIRECTORIES "${kyty_elf_dir}/include"
		INTERFACE_COMPILE_DEFINITIONS KYTY_RUST_ELF=1)
	add_dependencies(kyty_elf kyty_elf_build)
	add_test(NAME kyty_elf_rust
		COMMAND ${CMAKE_COMMAND} -E env "CARGO_TARGET_DIR=${kyty_elf_target_dir}"
		        "${KYTY_CARGO}" test --locked --manifest-path "${kyty_elf_dir}/Cargo.toml")
	# System libraries the Rust std needs (`cargo rustc -- --print native-static-libs`).
	if(WIN32)
		set_property(TARGET kyty_elf PROPERTY INTERFACE_LINK_LIBRARIES kernel32 ntdll userenv ws2_32 dbghelp)
	else()
		set_property(TARGET kyty_elf PROPERTY INTERFACE_LINK_LIBRARIES ${CMAKE_DL_LIBS} pthread m)
	endif()
endif()
