export interface Password {
  id: number;
  title: string;
  username: string;
  password: string;
  url?: string;
  notes?: string;
  created_at: number;
  updated_at: number;
}

export interface AppItem {
  id: number;
  name: string;
  path: string;
  icon?: string;
  arguments?: string;
  category_id?: number;
  custom_name?: string;
  created_at: number;
}

export interface AppCategory {
  id: number;
  name: string;
  icon?: string;
  sort_order: number;
}

export interface Snippet {
  id: number;
  title: string;
  content: string;
  language?: string;
  tags?: string;
  created_at: number;
  updated_at: number;
}

export interface TempContent {
  id: number;
  title?: string;
  content: string;
  expire_at: number;
  created_at: number;
}

export interface PinnedItem {
  id: number;
  type: "password" | "app" | "snippet" | "temp";
  item_id: number;
  sort_order: number;
}

export type NavItem =
  | { type: "password"; label: "密码区"; icon: string; count?: number }
  | { type: "app"; label: string; icon: string; categoryId: number; count?: number }
  | { type: "snippet"; label: "命令行"; icon: string; count?: number }
  | { type: "temp"; label: "临时内容"; icon: string; count?: number }
  | { type: "settings"; label: "设置"; icon: string; count?: number }
  | { type: "lock"; label: "锁定"; icon: string; count?: number };