import { knownAddresses } from "@/commands/wallet";
import { AddressRecord, NeptuneKeyType } from "@/utils/api/types";
import {
  ActionIcon,
  Box,
  Center,
  CopyButton,
  Flex,
  Loader,
  Paper,
  ScrollArea,
  Table,
  Tabs,
  Title,
  Tooltip,
} from "@mantine/core";
import { IconCheck, IconCopy } from "@tabler/icons-react";
import { useEffect, useState } from "react";

const generation_tab = "generation";
const viewing_tab = "viewing";
const ec_hybrid_tab = "echybrid";

export default function AddressesPage() {
  const [activeTab, setActiveTab] = useState<string | null>(generation_tab);
  const [addresses, setAddresses] = useState<AddressRecord[]>([]);
  const [isLoading, setIsLoading] = useState<boolean>(false);

  // This hook runs every time the `activeTab` changes
  useEffect(() => {
    async function fetchAddresses() {
      if (!activeTab) return;

      setIsLoading(true);

      try {
        let keyType: NeptuneKeyType = "Generation";
        if (activeTab === ec_hybrid_tab) keyType = "EcHybrid";
        if (activeTab === viewing_tab) keyType = "ViewingAddress";

        const data = await knownAddresses(keyType);

        setAddresses(data);
      } catch (error) {
        console.error("Failed to fetch addresses from backend:", error);
        // Optionally, handle error state here (e.g., setAddresses([]), show a notification)
      } finally {
        setIsLoading(false);
      }
    }

    fetchAddresses();
  }, [activeTab]); // The dependency array ensures this runs when activeTab changes

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
            {data.map((item) => (
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
      <Flex justify="space-between" align="center" mb="lg">
        <Title order={2} fw={500}>
          Addresses
        </Title>
      </Flex>

      <Paper withBorder radius="md" p="md">
        <Tabs value={activeTab} onChange={setActiveTab}>
          <Tabs.List mb="md">
            <Tabs.Tab value="generation">Generation</Tabs.Tab>
            <Tabs.Tab value="echybrid">EC hybrid</Tabs.Tab>
            <Tabs.Tab value="viewing">Viewing</Tabs.Tab>
          </Tabs.List>

          <Tabs.Panel value={activeTab || generation_tab}>
            <AddressTable data={addresses} />
          </Tabs.Panel>
        </Tabs>
      </Paper>
    </Box>
  );
}
