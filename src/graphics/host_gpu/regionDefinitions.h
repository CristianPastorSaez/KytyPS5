#ifndef EMULATOR_SRC_GRAPHICS_HOST_GPU_REGIONDEFINITIONS_H_
#define EMULATOR_SRC_GRAPHICS_HOST_GPU_REGIONDEFINITIONS_H_

#include "common/bitArray.h"
#include "common/common.h"
#include "kernel/memory.h"

#include <atomic>
#include <compare>
#include <cstdlib>
#include <new>

namespace Libs::Graphics {

constexpr uint64_t TRACKER_PAGE_SIZE    = 4ull * 1024ull;
constexpr uint64_t TRACKER_REGION_SIZE  = 4ull * 1024ull * 1024ull;
constexpr uint64_t TRACKER_ADDRESS_SIZE = 1ull << 44u;
constexpr uint64_t LOWER_ADDRESS_SIZE   = 1ull << 40u;
constexpr size_t   TRACKER_REGION_PAGES = TRACKER_REGION_SIZE / TRACKER_PAGE_SIZE;

struct GuestRange {
	uint64_t address = 0;
	uint64_t size    = 0;

	[[nodiscard]] constexpr bool Empty() const noexcept { return address == 0 && size == 0; }
	[[nodiscard]] constexpr bool Valid() const noexcept {
		if (address == 0 || size == 0) return false;
		if (address < LOWER_ADDRESS_SIZE) return size <= LOWER_ADDRESS_SIZE - address;
		constexpr auto base   = LibKernel::Memory::kExtendedMemoryBase;
		constexpr auto length = LibKernel::Memory::kExtendedMemorySize;
		return address >= base && address - base < length && size <= length - (address - base);
	}
	[[nodiscard]] constexpr bool     ValidOrEmpty() const noexcept { return Empty() || Valid(); }
	[[nodiscard]] constexpr uint64_t End() const noexcept { return address + size; }
	auto                             operator<=>(const GuestRange&) const = default;
};

enum class DirtySource { Cpu, Gpu };
using RegionBits = Common::BitArray<TRACKER_REGION_PAGES>;
static_assert(sizeof(RegionBits) == TRACKER_REGION_PAGES / 8);

// One lazily published pointer per tracked region (2^22 entries, 32 MiB). calloc returns fresh
// zeroed OS pages, so only pages holding published entries are ever touched; value-initialising
// std::atomic<T*>[] wrote all 32 MiB on every construction.
template <typename T>
class RegionTable final {
public:
	RegionTable(): m_entries(static_cast<T**>(std::calloc(COUNT, sizeof(T*)))) {
		if (m_entries == nullptr) {
			throw std::bad_alloc();
		}
	}
	~RegionTable() { std::free(m_entries); }
	KYTY_CLASS_NO_COPY(RegionTable);

	[[nodiscard]] T* Load(uint64_t index) const noexcept {
		return std::atomic_ref<T*>(m_entries[index]).load(std::memory_order_acquire);
	}
	void Store(uint64_t index, T* value) noexcept {
		std::atomic_ref<T*>(m_entries[index]).store(value, std::memory_order_release);
	}

private:
	static constexpr uint64_t COUNT = TRACKER_ADDRESS_SIZE / TRACKER_REGION_SIZE;
	T**                       m_entries;
};

} // namespace Libs::Graphics

#endif // EMULATOR_SRC_GRAPHICS_HOST_GPU_REGIONDEFINITIONS_H_
