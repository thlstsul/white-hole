use serde::{Deserialize, Serialize};

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct PageToken {
    pub limit: u32,
    pub offset: u32,
}

pub trait Paginator: Sized {
    fn as_limit_sql(&self) -> String;
    fn next_page<T>(&self, data: &mut Vec<T>) -> Option<Self>;
}

impl Paginator for PageToken {
    /// 多取一条数据
    fn as_limit_sql(&self) -> String {
        format!("LIMIT {} OFFSET {}", self.limit + 1, self.offset)
    }

    fn next_page<T>(&self, data: &mut Vec<T>) -> Option<Self> {
        if data.len() > self.limit as usize {
            data.pop();
            Some(Self {
                limit: self.limit,
                offset: self.offset + self.limit,
            })
        } else {
            None
        }
    }
}
