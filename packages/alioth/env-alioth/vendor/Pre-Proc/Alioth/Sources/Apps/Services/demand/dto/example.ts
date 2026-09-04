/**
 * RequirementItem 读模型（demand service，与后端 DTO 字段名 1:1）
 * 端点：GET/POST /service/demand/requirements、GET/PUT/DELETE /service/demand/requirements/{id}、
 *       GET /service/demand/requirements/dimensions
 * 列表响应：{ items: RequirementItem[], total: number }
 */
export interface RequirementItem {
  id: string;
  code: string | null;
  name: string;
  comments: string | null;
  category: string | null;
  place: string | null;
  createdAt: string;
  updatedAt: string | null;
  _refs?: {
    category?: { notice: string } | null;
    place?: { notice: string } | null;
  };
}

export interface CreateRequirementRequest {
  code: string;
  name: string;
  comments?: string | null;
  category?: string | null;
  place?: string | null;
}

export interface UpdateRequirementRequest {
  code?: string | null;
  name?: string | null;
  comments?: string | null;
  category?: string | null;
  place?: string | null;
}

export interface DimensionOption {
  id: string;
  name: string;
}

export interface DimensionsResponse {
  categories: DimensionOption[];
  places: DimensionOption[];
}
