// Toaster Mic PCM queue. Caller provides synchronization; no allocation or CRT.
#pragma once
template<unsigned Capacity> class ToasterRing {
    static_assert(Capacity % 4 == 0, "Whole stereo PCM16 frames required");
    unsigned char data_[Capacity]{};
    unsigned head_ = 0, size_ = 0;
public:
    void Reset() noexcept { head_ = size_ = 0; }
    unsigned Size() const noexcept { return size_; }
    bool Write(const unsigned char* src, unsigned bytes) noexcept {
        if (!src || bytes == 0 || bytes > Capacity || bytes % 4) return false;
        if (size_ + bytes > Capacity) {
            unsigned drop = size_ + bytes - Capacity;
            head_ = (head_ + drop) % Capacity; size_ -= drop;
        }
        for (unsigned i = 0; i < bytes; ++i) data_[(head_ + size_ + i) % Capacity] = src[i];
        size_ += bytes; return true;
    }
    void Read(unsigned char* dst, unsigned bytes) noexcept {
        unsigned take = bytes < size_ ? bytes : size_;
        for (unsigned i = 0; i < take; ++i) dst[i] = data_[(head_ + i) % Capacity];
        for (unsigned i = take; i < bytes; ++i) dst[i] = 0;
        head_ = (head_ + take) % Capacity; size_ -= take;
    }
};
