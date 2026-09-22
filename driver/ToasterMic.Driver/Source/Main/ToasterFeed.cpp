// Private input channel for the Toaster Mic Windows service. The public audio
// device is a normal WaveRT capture endpoint. Only SYSTEM/admin can feed it.
#include "ToasterFeed.h"
#include "ToasterRing.h"
#include <wdmsec.h>

static PDEVICE_OBJECT controlDevice;
static PDRIVER_DISPATCH previous[IRP_MJ_MAXIMUM_FUNCTION + 1];
static KSPIN_LOCK queueLock;
static ToasterRing<5760> queue; // 30 ms maximum; newest audio wins on overrun.
static ULONGLONG lastFeed;
static const GUID controlClass = {0x30b65426,0xe718,0x4762,{0xb3,0x8b,0x21,0xc8,0xb8,0xa6,0xed,0xb3}};
static constexpr ULONG feedIoctl = CTL_CODE(FILE_DEVICE_UNKNOWN, 0x801, METHOD_BUFFERED, FILE_WRITE_DATA);
static constexpr ULONG resetIoctl = CTL_CODE(FILE_DEVICE_UNKNOWN, 0x802, METHOD_BUFFERED, FILE_WRITE_DATA);

static void ResetQueue() {
    KIRQL irql; KeAcquireSpinLock(&queueLock, &irql);
    queue.Reset(); lastFeed = 0; KeReleaseSpinLock(&queueLock, irql);
}

_Use_decl_annotations_
static NTSTATUS Dispatch(PDEVICE_OBJECT device, PIRP irp) {
    auto stack = IoGetCurrentIrpStackLocation(irp);
    if (device != controlDevice) return previous[stack->MajorFunction](device, irp);
    NTSTATUS status = STATUS_INVALID_DEVICE_REQUEST;
    switch (stack->MajorFunction) {
    case IRP_MJ_CREATE:
        if (stack->FileObject->FileName.Length) { status = STATUS_OBJECT_NAME_NOT_FOUND; break; }
        ResetQueue(); status = STATUS_SUCCESS; break;
    case IRP_MJ_CLEANUP:
    case IRP_MJ_CLOSE:
        ResetQueue(); status = STATUS_SUCCESS; break;
    case IRP_MJ_DEVICE_CONTROL: {
        auto& request = stack->Parameters.DeviceIoControl;
        if (request.IoControlCode == resetIoctl && request.InputBufferLength == 0) {
            ResetQueue(); status = STATUS_SUCCESS;
        } else if (request.IoControlCode == feedIoctl && request.InputBufferLength == 960 && irp->AssociatedIrp.SystemBuffer) {
            KIRQL irql; KeAcquireSpinLock(&queueLock, &irql);
            queue.Write(static_cast<unsigned char*>(irp->AssociatedIrp.SystemBuffer), 960);
            lastFeed = KeQueryInterruptTime(); KeReleaseSpinLock(&queueLock, irql);
            status = STATUS_SUCCESS;
        } else status = STATUS_INVALID_PARAMETER;
        break;
    }
    }
    irp->IoStatus.Status = status; irp->IoStatus.Information = 0;
    IoCompleteRequest(irp, IO_NO_INCREMENT); return status;
}

NTSTATUS ToasterFeedInitialize(PDRIVER_OBJECT driver) {
    KeInitializeSpinLock(&queueLock); ResetQueue();
    UNICODE_STRING name = RTL_CONSTANT_STRING(L"\\Device\\ToasterMicFeed");
    UNICODE_STRING link = RTL_CONSTANT_STRING(L"\\DosDevices\\ToasterMicFeed");
    UNICODE_STRING sddl = RTL_CONSTANT_STRING(L"D:P(A;;GA;;;SY)(A;;GA;;;BA)");
    NTSTATUS status = IoCreateDeviceSecure(driver, 0, &name, FILE_DEVICE_UNKNOWN,
        FILE_DEVICE_SECURE_OPEN, TRUE, &sddl, &controlClass, &controlDevice);
    if (!NT_SUCCESS(status)) return status;
    status = IoCreateSymbolicLink(&link, &name);
    if (!NT_SUCCESS(status)) { IoDeleteDevice(controlDevice); controlDevice = nullptr; return status; }
    ULONG majors[] = {IRP_MJ_CREATE, IRP_MJ_CLOSE, IRP_MJ_CLEANUP, IRP_MJ_DEVICE_CONTROL};
    for (ULONG major : majors) {
        previous[major] = driver->MajorFunction[major]; driver->MajorFunction[major] = Dispatch;
    }
    controlDevice->Flags &= ~DO_DEVICE_INITIALIZING;
    return STATUS_SUCCESS;
}

void ToasterFeedShutdown() {
    if (!controlDevice) return;
    UNICODE_STRING link = RTL_CONSTANT_STRING(L"\\DosDevices\\ToasterMicFeed");
    IoDeleteSymbolicLink(&link); IoDeleteDevice(controlDevice); controlDevice = nullptr;
}

// Called from the existing WaveRT DPC; this code and queue stay in nonpaged memory.
void ToasterFeedRead(PUCHAR destination, ULONG bytes) {
    KIRQL irql; KeAcquireSpinLock(&queueLock, &irql);
    if (KeQueryInterruptTime() - lastFeed > 3000000ULL) queue.Reset(); // 300 ms stale-data ceiling
    queue.Read(destination, bytes);
    KeReleaseSpinLock(&queueLock, irql);
}
