import { generateNewAddress, knownAddresses } from "@/commands/wallet";
import { AddressRecord, NeptuneKeyType } from "@/utils/api/types";
import {
  ActionIcon,
  Box,
  Button,
  Center,
  CopyButton,
  Flex,
  Loader,
  Paper,
  ScrollArea,
  Table,
  Tabs,
  Text,
  Title,
  Tooltip,
} from "@mantine/core";
import { IconCheck, IconCopy, IconPlus } from "@tabler/icons-react";
import { useCallback, useEffect, useState } from "react";

const generation_tab = "generation";
const viewing_tab = "viewing";
const ec_hybrid_tab = "echybrid";

export default function AddressesPage() {
  const [activeTab, setActiveTab] = useState<string | null>(generation_tab);
  const [addresses, setAddresses] = useState<AddressRecord[]>([]);
  const [isLoading, setIsLoading] = useState<boolean>(false);
  const [isGenerating, setIsGenerating] = useState<boolean>(false);

  const BUTTON_LABELS: Record<string, string> = {
    [generation_tab]: "New Generation Address",
    [ec_hybrid_tab]: "New EC Hybrid Address",
    [viewing_tab]: "New Viewing Address",
  };

  const ADDRESS_DESCRIPTIONS: Record<string, string> = {
    [generation_tab]:
      "Generation addresses: Will not leak privacy if you reuse it and share it with multiple people.",
    [ec_hybrid_tab]:
      "EC hybrid addresses: It's recommended to only share each address with one other party. Otherwise, an attacker with a powerful quantum computer might expose (but not steal) your incoming transactions.",
    [viewing_tab]:
      "Viewing address: Only share each address with one other party. Anyone seeing one of your addresses can see anything that address has ever received.",
  };

  const keyTypeFromTab = (tab: string | null): NeptuneKeyType => {
    if (tab === ec_hybrid_tab) return "EcHybrid";
    if (tab === viewing_tab) return "ViewingAddress";
    return "Generation";
  };

  const fetchAddresses = useCallback(async () => {
    if (!activeTab) return;
    setIsLoading(true);
    try {
      const keyType = keyTypeFromTab(activeTab);
      const data = await knownAddresses(keyType);
      setAddresses(data);
    } catch (error) {
      console.error("Failed to fetch addresses from backend:", error);
    } finally {
      setIsLoading(false);
    }
  }, [activeTab]);

  useEffect(() => {
    fetchAddresses();
  }, [fetchAddresses]);

  // Handler for the generate button
  const handleGenerate = async () => {
    if (!activeTab) return;
    setIsGenerating(true);
    try {
      const keyType = keyTypeFromTab(activeTab);
      const newAddress = await generateNewAddress(keyType);

      // Append the new address to the existing list without needing a full refetch
      setAddresses((prev) => [...prev, newAddress]);
    } catch (error) {
      console.error("Failed to generate new address:", error);
    } finally {
      setIsGenerating(false);
    }
  };

  const addressRepresentation = (address: AddressRecord): string =>
    activeTab === generation_tab ? address.address_short_form : address.address;

  const AddressTable = ({ data }: { data: AddressRecord[] }) => {
    // Show a spinner while Tauri is fetching
    if (isLoading) {
      return (
        <Center p="xl">
          <Loader color="blue" />
        </Center>
      );
    }

    if (data.length === 0) {
      return (
        <Box p="md" ta="center" c="dimmed">
          No addresses found.
        </Box>
      );
    }

    // Sort the data in reverse chronological order, showing the address with
    // the highest index first.
    const sortedData = [...data].sort((a, b) => b.key_index - a.key_index);

    return (
      <ScrollArea h="calc(100vh - 220px)" type="auto" offsetScrollbars>
        <Table verticalSpacing="sm" striped highlightOnHover>
          <Table.Thead
            style={{
              position: "sticky",
              top: 0,
              backgroundColor: "var(--mantine-color-body)",
              zIndex: 1,
            }}
          >
            <Table.Tr>
              <Table.Th>Key index</Table.Th>
              <Table.Th>Address</Table.Th>
              <Table.Th w={80} ta="right">
                Action
              </Table.Th>
            </Table.Tr>
          </Table.Thead>
          <Table.Tbody>
            {sortedData.map((item) => (
              <Table.Tr key={item.key_index}>
                <Table.Td>{item.key_index}</Table.Td>
                <Table.Td>
                  <Box style={{ wordBreak: "break-all" }}>{addressRepresentation(item)}</Box>
                </Table.Td>
                <Table.Td ta="right">
                  <CopyButton value={item.address} timeout={2000}>
                    {({ copied, copy }) => (
                      <Tooltip label={copied ? "Copied" : "Copy"} withArrow position="right">
                        <ActionIcon
                          color={copied ? "teal" : "gray"}
                          variant="subtle"
                          onClick={copy}
                        >
                          {copied ? <IconCheck size={16} /> : <IconCopy size={16} />}
                        </ActionIcon>
                      </Tooltip>
                    )}
                  </CopyButton>
                </Table.Td>
              </Table.Tr>
            ))}
          </Table.Tbody>
        </Table>
      </ScrollArea>
    );
  };

  return (
    <Box p="md">
      <Title order={2} fw={500}>
        Addresses
      </Title>

      <Paper withBorder radius="md" p="md">
        <Tabs value={activeTab} onChange={setActiveTab}>
          <Tabs.List mb="md">
            <Tabs.Tab value="generation">Generation</Tabs.Tab>
            <Tabs.Tab value="echybrid">EC hybrid</Tabs.Tab>
            <Tabs.Tab value="viewing">Viewing</Tabs.Tab>
          </Tabs.List>

          <Tabs.Panel value={activeTab || generation_tab}>
            <Flex justify="space-between" align="center" mb="sm" wrap="wrap" gap="sm">
              <Text c="dimmed" size="sm" style={{ flex: 1 }}>
                {activeTab ? ADDRESS_DESCRIPTIONS[activeTab] : ""}
              </Text>

              <Button
                leftSection={<IconPlus size={15} />}
                onClick={handleGenerate}
                loading={isGenerating}
              >
                {activeTab ? BUTTON_LABELS[activeTab] : "Generate New Address"}
              </Button>
            </Flex>

            <AddressTable data={addresses} />
          </Tabs.Panel>
        </Tabs>
      </Paper>
    </Box>
  );
}
