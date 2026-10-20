// Public fixture calculated independently with Python urllib.parse/hashlib/hmac.
pub const URI: &str = r###"/bucket/ZZZZZZZZ%20key/%E9%9B%AA"###;
pub const QUERY: &str = r###"a=%E9%9B%AA&a=a%2Fb%20c&z=ZZZZZZZZ"###;
pub const CANONICAL_REQUEST: &str = r###"GET
/bucket/ZZZZZZZZ%20key/%E9%9B%AA
a=%E9%9B%AA&a=a%2Fb%20c&z=ZZZZZZZZ
host:example.test
x-amz-content-sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
x-amz-date:20261020T120000Z
x-zzzzzzzz:private value 雪

host;x-amz-content-sha256;x-amz-date;x-zzzzzzzz
e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"###;
pub const STRING_TO_SIGN: &str = r###"AWS4-HMAC-SHA256
20261020T120000Z
20261020/us-east-1/s3/aws4_request
6c96ea838cdb1e631d840970d0973295b1037e545b2c3d749c95f97ce0f216cd"###;
pub const SIGNED_HEADERS: &str = r###"host;x-amz-content-sha256;x-amz-date;x-zzzzzzzz"###;
pub const SIGNATURE: &str =
    r###"85a63028b8a2ab4533a35babb4889ff3bccb259cbc3c3c6ccecac0c11df58afd"###;
pub const AUTHORIZATION: &str = r###"AWS4-HMAC-SHA256 Credential=PUBLIC/20261020/us-east-1/s3/aws4_request, SignedHeaders=host;x-amz-content-sha256;x-amz-date;x-zzzzzzzz, Signature=85a63028b8a2ab4533a35babb4889ff3bccb259cbc3c3c6ccecac0c11df58afd"###;
