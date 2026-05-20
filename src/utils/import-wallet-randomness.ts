import { importIncomingRandomness } from "@/commands/wallet";
import { open } from "@tauri-apps/plugin-dialog";
import { readTextFile } from "@tauri-apps/plugin-fs";

export interface IncomingUtxoRecoveryData {
  utxo: any;
  sender_randomness: string;
  receiver_preimage: string;
  aocl_index: number;
}

export async function handleImportRandomness(): Promise<void> {
  try {
    // 1. Open Tauri file dialog
    const selectedPath = await open({
      multiple: false,
      filters: [
        {
          name: "Neptune Incoming Randomness Data",
          extensions: ["dat"],
        },
      ],
    });

    if (!selectedPath || Array.isArray(selectedPath)) {
      // User canceled the dialog
      return;
    }

    // 2. Read the file contents
    const fileContents = await readTextFile(selectedPath);

    // 3. Parse the JSON Lines (JSONL) data
    const parsedData: IncomingUtxoRecoveryData[] = fileContents
      .split(/\r?\n/) // Split by newline, handles both \n and \r\n
      .filter((line) => line.trim() !== "") // Ignore empty lines (prevents JSON syntax errors)
      .map((line) => JSON.parse(line)); // Parse each individual line into an object

    console.log("Successfully imported randomness:", parsedData);

    // 4. Pass the data back to your Rust backend for processing
    const _res = await importIncomingRandomness(parsedData);

    // TODO: Add a Mantine Notification here for success!
  } catch (error) {
    console.error("Failed to import randomness:", error);
    // TODO: Add a Mantine Notification here for the error
  }
}
