#pragma once
#include <ntddk.h>
NTSTATUS ToasterFeedInitialize(PDRIVER_OBJECT driver);
void ToasterFeedShutdown();
void ToasterFeedRead(PUCHAR destination, ULONG bytes);
